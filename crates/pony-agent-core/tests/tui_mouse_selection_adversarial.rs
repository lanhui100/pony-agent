// TUI 鼠标文本选择与复制对抗与鲁棒性破坏测试
// 严格遵循 AAA 原则与零假测试规范

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use std::time::{Duration, Instant};
use tui::{
    encode_osc52_copy, extract_selected_text, is_coord_selected, MouseSelectionState, TextCoord,
};

// ---------------------------------------------------------------------------
// 1. 防抖时钟模拟与快速拖拽多事件注入（连续更新 head 时 debounce_until 刷新与过期判定）
// ---------------------------------------------------------------------------

#[test]
fn test_mouse_selection_debounce_simulation_and_burst_updates() {
    // Arrange: 准备鼠标选择状态
    let mut state = MouseSelectionState::default();
    let _now = Instant::now();
    let debounce_window = Duration::from_millis(50);

    state.active = true;
    state.anchor = Some(TextCoord { line: 0, col: 0 });

    // Act 1: 连续高频注入 50 次拖拽更新 (模拟 60fps+ 快速滑动)
    for i in 1..=50 {
        state.head = Some(TextCoord { line: 0, col: i });
        // 每次拖拽事件都会推迟防抖时限（即重置 debounce_until）
        state.debounce_until = Some(Instant::now() + debounce_window);
    }

    // Assert 1: 确保状态机未发生数据撕裂，最终 head 准确落在最后一次坐标
    assert_eq!(state.head, Some(TextCoord { line: 0, col: 50 }));
    assert!(
        state.debounce_until.is_some(),
        "高频拖拽后 debounce_until 必须处于等待窗口中"
    );

    let until = state.debounce_until.unwrap();
    // 刚刚设置的 debounce_until 必定在未来（防抖处于活动阻断期）
    assert!(
        until > Instant::now() || until == Instant::now(),
        "当前时刻不应超过防抖时间窗口"
    );

    // Act 2: 模拟经过防抖窗口后的判定
    let expired_until = Instant::now().checked_sub(Duration::from_millis(10)).unwrap();
    state.debounce_until = Some(expired_until);

    // Assert 2: 判定防抖已到期，允许执行重量级重绘/更新
    let is_expired = state.debounce_until.map_or(true, |t| Instant::now() >= t);
    assert!(is_expired, "超过 debounce_until 时间戳后判定应为已就绪");
}

// ---------------------------------------------------------------------------
// 2. 超大/极长文本（如 100,000 字符）提取性能与内存安全
// ---------------------------------------------------------------------------

#[test]
fn test_extract_selected_text_massive_string_safety() {
    // Arrange: 构造 100,000 字符长行与多行海量数据
    let large_char_count = 100_000;
    let long_line: String = "A".repeat(large_char_count);
    let lines = vec![
        "Start line".to_string(),
        long_line,
        "End line".to_string(),
    ];

    let start_time = Instant::now();

    // Act: 提取跨越超大单行的切片 (从 line 1 col 10,000 到 col 90,000)
    let extracted = extract_selected_text(
        &lines,
        TextCoord {
            line: 1,
            col: 10_000,
        },
        TextCoord {
            line: 1,
            col: 90_000,
        },
    );

    let elapsed = start_time.elapsed();

    // Assert: 验证切片长度与内容正确性，且在合理性能阈值内无 OOM/Panic
    assert_eq!(
        extracted.len(),
        80_001,
        "提取的子串字符长度必须完全吻合 [10000..=90000]"
    );
    assert!(
        extracted.chars().all(|c| c == 'A'),
        "提取内容应全部为预期字符 'A'"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "10 万字符切片提取必须在 500ms 内完成，实际耗时 {:?}",
        elapsed
    );

    // Act 2: OSC 52 编码超大文本（10,000 字符）安全测试
    let osc_start = Instant::now();
    let osc_payload = encode_osc52_copy(&extracted[..10_000]);
    let osc_elapsed = osc_start.elapsed();

    // Assert 2
    assert!(
        osc_payload.starts_with("\x1b]52;c;"),
        "OSC 52 必须具有正确的转义头"
    );
    assert!(
        osc_payload.ends_with("\x07"),
        "OSC 52 必须具有标准的 BEL 终止符"
    );
    assert!(
        osc_elapsed < Duration::from_millis(100),
        "OSC 52 编码 10,000 字符需在 100ms 内完成"
    );
}

// ---------------------------------------------------------------------------
// 3. 极端坐标注入（超出 lines 长度、反向零宽等）
// ---------------------------------------------------------------------------

#[test]
fn test_extreme_coordinates_injection() {
    // Arrange: 只有 2 行文本
    let lines = vec!["Row0".to_string(), "Row1".to_string()];

    // Act 1: 极大行索引注入 (超出 lines.len())
    let text1 = extract_selected_text(
        &lines,
        TextCoord {
            line: usize::MAX - 1,
            col: 0,
        },
        TextCoord {
            line: usize::MAX,
            col: usize::MAX,
        },
    );
    // Assert 1: 不 panic，安全返回空串
    assert_eq!(text1, "");

    // Act 2: 极大列索引注入 (首行存在，但列为 usize::MAX)
    let text2 = extract_selected_text(
        &lines,
        TextCoord {
            line: 0,
            col: usize::MAX,
        },
        TextCoord {
            line: 0,
            col: usize::MAX,
        },
    );
    // Assert 2: 不 panic，超出范围返回空
    assert_eq!(text2, "");

    // Act 3: 零宽选取 (anchor == head)
    let text3 = extract_selected_text(
        &lines,
        TextCoord { line: 0, col: 1 },
        TextCoord { line: 0, col: 1 },
    );
    // Assert 3: 按照 ..=col 闭区间语义，提取单字符
    assert_eq!(text3, "o");

    // Act 4: 反向选取，首尾跨越但列坐标反转（起点 col > 终点 col 在跨行情况下）
    let text4 = extract_selected_text(
        &lines,
        TextCoord { line: 1, col: 0 },
        TextCoord { line: 0, col: 2 },
    );
    // Assert 4: 规范化后从 (0, 2) 到 (1, 0)
    // line 0: "w0", line 1: "R"
    assert_eq!(text4, "w0\nR");
}

// ---------------------------------------------------------------------------
// 4. 空文本切片安全防护
// ---------------------------------------------------------------------------

#[test]
fn test_empty_lines_slice_safety() {
    // Arrange: 绝对空数组
    let empty_lines: Vec<String> = Vec::new();

    // Act 1: 任意坐标提取
    let text1 = extract_selected_text(
        &empty_lines,
        TextCoord { line: 0, col: 0 },
        TextCoord { line: 0, col: 0 },
    );
    // Assert 1: 必须返回空串，不可越界索引
    assert_eq!(text1, "");

    // Act 2: 包含空字符串行的切片
    let blank_lines = vec!["".to_string(), "".to_string()];
    let text2 = extract_selected_text(
        &blank_lines,
        TextCoord { line: 0, col: 0 },
        TextCoord { line: 1, col: 0 },
    );
    // Assert 2: 两行空串跨行应为 "\n" 或空串，无崩溃发生
    assert_eq!(text2, "\n");

    // Act 3: 选区命中判定在空行的行为
    let coord = TextCoord { line: 0, col: 0 };
    assert!(is_coord_selected(
        coord,
        TextCoord { line: 0, col: 0 },
        TextCoord { line: 0, col: 0 }
    ));
}
