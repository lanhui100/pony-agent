//! 原 `tui.rs` 内联测试模块 `mod splash_gradient_tests`（Step1 机械外移，断言零改动）。

use super::*;

fn rgb_at(t: f32) -> [u8; 3] {
    splash_gradient_rgb(t)
}

#[test]
fn splash_logo_mask_shapes_up() {
    assert_eq!(SPLASH_LOGO_ROWS, 22, "折角标像素网格行数");
    assert_eq!(SPLASH_LOGO_COLS, 28, "折角标像素网格列数");
    for (r, row) in SPLASH_LOGO_SHAPE.iter().enumerate() {
        assert_eq!(row.len(), SPLASH_LOGO_COLS, "第 {r} 行掩码长度不符");
        assert!(
            row.bytes().all(|b| b == b'#' || b == b'.'),
            "第 {r} 行掩码只允许 '#' / '.'"
        );
    }
}

#[test]
fn splash_logo_asset_decodes_for_image_protocol() {
    let img = image::load_from_memory(SPLASH_LOGO_PNG).expect("内嵌品牌标必须可解码");
    assert_eq!((img.width(), img.height()), (616, 484));
}

#[test]
fn splash_image_placeholder_matches_banner_grid() {
    assert_eq!(SPLASH_IMAGE_COLS, SPLASH_LOGO_COLS as u16);
    assert_eq!(SPLASH_IMAGE_ROWS, (SPLASH_LOGO_ROWS / 2) as u16);
}

fn probe_env(
    term: Option<&str>,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
    kitty_window_id: Option<&str>,
    multiplexer: Option<&str>,
) -> SplashEnv {
    SplashEnv {
        term: term.map(str::to_string),
        term_program: term_program.map(str::to_string),
        lc_terminal: lc_terminal.map(str::to_string),
        kitty_window_id: kitty_window_id.map(str::to_string),
        multiplexer: multiplexer.map(str::to_string),
    }
}

/// 启动期协议判定必须是**纯环境表**（零 stdin 读）且 fail-closed：
/// 拿不到强信号一律 `None`（自绘字符画），sixel 只认明确声明支持的终端。
#[test]
fn splash_protocol_detection_is_env_only_and_fail_closed() {
    use ProtocolType::{Iterm2, Kitty, Sixel};
    let cases: [(&str, SplashEnv, Option<ProtocolType>); 13] = [
        (
            "裸 xterm 无任何图形能力信号 → 字符画",
            probe_env(Some("xterm-256color"), None, None, None, None),
            None,
        ),
        (
            "TERM 缺失 → 字符画",
            probe_env(None, None, None, None, None),
            None,
        ),
        (
            "KITTY_WINDOW_ID 命中 kitty 档",
            probe_env(Some("xterm-256color"), None, None, Some("1"), None),
            Some(Kitty),
        ),
        (
            "TERM=xterm-kitty",
            probe_env(Some("xterm-kitty"), None, None, None, None),
            Some(Kitty),
        ),
        (
            "TERM_PROGRAM=ghostty",
            probe_env(
                Some("xterm-ghostty"),
                Some("ghostty"),
                None,
                None,
                None,
            ),
            Some(Kitty),
        ),
        (
            "TERM_PROGRAM=WezTerm",
            probe_env(Some("xterm-256color"), Some("WezTerm"), None, None, None),
            Some(Kitty),
        ),
        (
            "TERM_PROGRAM=iTerm.app",
            probe_env(
                Some("xterm-256color"),
                Some("iTerm.app"),
                None,
                None,
                None,
            ),
            Some(Iterm2),
        ),
        (
            "LC_TERMINAL=iTerm2",
            probe_env(Some("xterm-256color"), None, Some("iTerm2"), None, None),
            Some(Iterm2),
        ),
        (
            "TERM=foot（明确 sixel）",
            probe_env(Some("foot"), None, None, None, None),
            Some(Sixel),
        ),
        (
            "TERM 显式含 sixel",
            probe_env(Some("xterm-256color-sixel"), None, None, None, None),
            Some(Sixel),
        ),
        (
            "复用器压制 kitty 强信号",
            probe_env(Some("xterm-kitty"), None, None, Some("1"), Some("TMUX")),
            None,
        ),
        (
            "herdr 压制 sixel",
            probe_env(Some("foot"), None, None, None, Some("HERDR")),
            None,
        ),
        (
            "大小写不敏感",
            probe_env(Some("XTERM-KITTY"), None, None, None, None),
            Some(Kitty),
        ),
    ];
    for (label, env, expected) in cases {
        assert_eq!(
            splash_protocol_for_env(&env),
            expected,
            "用例 `{label}` 判定不符"
        );
    }
}

/// 探测链路不得再调用终端能力查询（它会阻塞读 stdin 并自行关掉 raw mode，是启动期
/// 吃键的根因）。断言禁止的是**调用形态**，故 needle 在运行时拼接，避免与本测试自身的
/// 说明文字互相命中。
#[test]
fn splash_probe_never_reads_stdin() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/tui.rs");
    let source = std::fs::read_to_string(path).expect("读取 tui.rs 源码");
    let forbidden = ["from_query", "_stdio("].concat();
    assert!(
        !source.contains(&forbidden),
        "启动期品牌标探测必须保持零 stdin 读：`{forbidden}` 会吞用户按键并关闭 raw mode"
    );
}

#[test]
fn splash_gradient_covers_full_logo_diagonal() {
    // 归一化必须让折角标真实的两个对角极值恰好落在 t=0 / t=1；
    // 否则首尾亮部色标被裁掉（旧实现浪费约 10%）。
    let (mut min_t, mut max_t) = (f32::MAX, f32::MIN);
    for (y, row) in SPLASH_LOGO_SHAPE.iter().enumerate() {
        for (x, cell) in row.bytes().enumerate() {
            if cell != b'#' {
                continue;
            }
            let t = splash_diagonal_t(x, y);
            min_t = min_t.min(t);
            max_t = max_t.max(t);
        }
    }
    assert!(
        min_t <= 0.01,
        "折角标最小对角位置应归一化到 0，实际 {min_t}"
    );
    assert!(
        max_t >= 0.99,
        "折角标最大对角位置应归一化到 1，实际 {max_t}"
    );
}

#[test]
fn splash_gradient_keeps_bright_highlights() {
    // 「加入亮色」的可验证判据（OKLab 亮度）：起点暖金高光 ≥ 0.85
    // （旧版橙 #f97316 约 0.70），收尾靛蓝亮度高于旧版 #4f46e5（约 0.51）。
    assert_eq!(rgb_at(0.0), [255, 214, 103], "起点必须是暖金高光色标");
    assert_eq!(rgb_at(1.0), [91, 91, 240], "终点必须是提亮后的靛蓝收尾");
    let start = srgb_to_oklab(rgb_at(0.0));
    assert!(start[0] >= 0.85, "暖金高光亮度不足: {start:?}");
    let end = srgb_to_oklab(rgb_at(1.0));
    assert!(end[0] > 0.53, "靛蓝收尾未提亮: {end:?}");
    // 全程保持彩度，红→紫中段不塌成发灰暗带。
    let mut min_chroma = f32::MAX;
    for step in 0..=128 {
        let lab = srgb_to_oklab(rgb_at(step as f32 / 128.0));
        min_chroma = min_chroma.min((lab[1] * lab[1] + lab[2] * lab[2]).sqrt());
    }
    assert!(
        min_chroma >= 0.12,
        "渐变出现低彩度灰带: min_chroma={min_chroma:.3}"
    );
}

#[test]
fn splash_gradient_steps_are_smooth() {
    // 1/256 采样下每通道跳变 ≤ 6/255：16 行画布逐行过渡，不出现硬色带。
    let mut prev = rgb_at(0.0);
    for step in 1..=256 {
        let t = step as f32 / 256.0;
        let cur = rgb_at(t);
        for (channel, (a, b)) in prev.iter().zip(cur.iter()).enumerate() {
            let delta = (*a as i32 - *b as i32).abs();
            assert!(
                delta <= 6,
                "t={t:.4} 通道 {channel} 跳变 {delta}: {prev:?} -> {cur:?}"
            );
        }
        prev = cur;
    }
}
