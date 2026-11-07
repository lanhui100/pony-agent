//! 原 `tui.rs` 内联测试模块 `mod splash_gradient_tests`（Step1 机械外移）。
//! ADR 0085 落地后扩为「生成物来源/尺寸 + AA 合成 + 真彩门禁 + 尺寸阶梯」一组门禁。

use super::*;

fn rgb_at(t: f32) -> [u8; 3] {
    splash_gradient_rgb(t)
}

// ── 生成物来源与尺寸（覆盖率表）──────────────────────────────────────────────

/// 覆盖率表必须与生成脚本声明的档位尺寸严格一致，且带「禁止手改 / 来源 master」头注释。
/// 这是「单一真值源 = 官方母版重采样」的编译期 + 运行期双重锚点。
#[test]
fn splash_coverage_tables_match_declared_dims() {
    assert_eq!(splash_coverage::RICH_COLS, 27, "Rich 档列数");
    assert_eq!(splash_coverage::RICH_ROWS, 30, "Rich 档子像素行数");
    assert_eq!(splash_coverage::COMPACT_COLS, 20, "Compact 档列数");
    assert_eq!(splash_coverage::COMPACT_ROWS, 22, "Compact 档子像素行数");

    assert_eq!(
        splash_coverage::RICH_COVERAGE.len(),
        splash_coverage::RICH_COLS * splash_coverage::RICH_ROWS,
        "Rich 覆盖率表长度必须 = 列 × 子像素行"
    );
    assert_eq!(
        splash_coverage::COMPACT_COVERAGE.len(),
        splash_coverage::COMPACT_COLS * splash_coverage::COMPACT_ROWS,
        "Compact 覆盖率表长度必须 = 列 × 子像素行"
    );
    // 子像素行必须为偶数（一个半块字符 = 上下 2 行）。
    assert_eq!(splash_coverage::RICH_ROWS % 2, 0);
    assert_eq!(splash_coverage::COMPACT_ROWS % 2, 0);

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/tui/splash_coverage.rs");
    let source = std::fs::read_to_string(path).expect("读取覆盖率表源码");
    assert!(
        source.contains("icon-master-approved.png"),
        "覆盖率表必须标注母版来源（防漂移）"
    );
    assert!(
        source.contains("禁止手改"),
        "覆盖率表必须带「禁止手改」生成头注释"
    );
    assert!(
        source.contains("RICH_COVERAGE") && source.contains("COMPACT_COVERAGE"),
        "覆盖率表必须同时导出 Rich / Compact 两档"
    );
}

#[test]
fn splash_logo_asset_decodes_for_image_protocol() {
    // 位图由同一 master 生成，铺满 Rich 档字符矩形（27×15 格 × 8×16 像素 × 4 超采样）。
    let img = image::load_from_memory(SPLASH_LOGO_PNG).expect("内嵌品牌标必须可解码");
    assert_eq!((img.width(), img.height()), (864, 960));
}

#[test]
fn splash_image_placeholder_matches_rich_tier() {
    assert_eq!(SPLASH_IMAGE_COLS, splash_coverage::RICH_COLS as u16);
    assert_eq!(
        SPLASH_IMAGE_ROWS,
        (splash_coverage::RICH_ROWS / 2) as u16
    );
}

// ── 协议判定：纯环境表、fail-closed、零 stdin 读 ─────────────────────────────

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

// ── 渐变投影 / 色标 / 平滑 ──────────────────────────────────────────────────

#[test]
fn splash_diagonal_projection_spans_bbox() {
    // `t` 为标记 bbox 的 45° 轴投影：左上角 = 0（暖金端），右下角 = 1（紫端）。
    let cols = splash_coverage::RICH_COLS;
    let rows = splash_coverage::RICH_ROWS;
    assert!((splash_diagonal_t(0, 0, cols, rows) - 0.0).abs() < 1e-6);
    assert!((splash_diagonal_t(cols - 1, rows - 1, cols, rows) - 1.0).abs() < 1e-6);
    // x、y 各自单调递增。
    let mut prev = -1.0f32;
    for x in 0..cols {
        let t = splash_diagonal_t(x, 0, cols, rows);
        assert!(t >= prev, "沿 x 必须单调不减");
        prev = t;
    }
    let mut prev = -1.0f32;
    for y in 0..rows {
        let t = splash_diagonal_t(0, y, cols, rows);
        assert!(t >= prev, "沿 y 必须单调不减");
        prev = t;
    }
}

#[test]
fn splash_gradient_uses_c2_endpoints() {
    // C2 克制暖色三档：暖金 #ffc478 → 赤陶 #e96e54 → 紫 #965cdc。
    assert_eq!(rgb_at(0.0), [255, 196, 120], "起点必须是暖金高光色标");
    assert_eq!(rgb_at(1.0), [150, 92, 220], "终点必须是紫色标");
    let start = srgb_to_oklab(rgb_at(0.0));
    assert!(start[0] >= 0.85, "暖金高光亮度不足: {start:?}");
    let end = srgb_to_oklab(rgb_at(1.0));
    assert!(end[0] > 0.55, "紫色收尾过暗: {end:?}");
    // 全程保持彩度，暖金→赤陶→紫中段不塌成发灰暗带。
    let mut min_chroma = f32::MAX;
    for step in 0..=256 {
        let lab = srgb_to_oklab(rgb_at(step as f32 / 256.0));
        min_chroma = min_chroma.min((lab[1] * lab[1] + lab[2] * lab[2]).sqrt());
    }
    assert!(
        min_chroma >= 0.11,
        "渐变出现低彩度灰带: min_chroma={min_chroma:.3}"
    );
}

#[test]
fn splash_gradient_steps_are_smooth() {
    // 1/256 采样下每通道跳变 ≤ 6/255：逐行过渡，不出现硬色带。
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

// ── 真彩门禁 / 覆盖率 alpha 合成 ───────────────────────────────────────────

#[test]
fn splash_colorterm_truecolor_gate() {
    assert!(colorterm_supports_truecolor(Some("truecolor")));
    assert!(colorterm_supports_truecolor(Some("24bit")));
    assert!(colorterm_supports_truecolor(Some("TRUECOLOR")));
    assert!(!colorterm_supports_truecolor(Some("256")));
    assert!(!colorterm_supports_truecolor(Some("")));
    assert!(!colorterm_supports_truecolor(None));
}

/// 非真彩不得产生任何 AA 混合色：每半格只能是「纯渐变色」或「画布色」二者之一。
#[test]
fn splash_composite_hard_threshold_has_no_aa() {
    let grad = [200u8, 120, 90];
    let canvas = [18u8, 16, 14];
    for cov in 0u8..=255 {
        let out = splash_composite_color(grad, cov, false);
        assert!(
            out == grad || out == canvas,
            "非真彩 cov={cov} 产生了 AA 色: {out:?}"
        );
        if cov >= 128 {
            assert_eq!(out, grad, "非真彩 ≥50% 覆盖应上满色");
        } else {
            assert_eq!(out, canvas, "非真彩 <50% 覆盖应回落画布色");
        }
    }
    // 真彩下同一覆盖率会产生介于画布与纯色之间的混合色（边缘渐隐）。
    let mid = splash_composite_color(grad, 128, true);
    assert_ne!(mid, grad);
    assert_ne!(mid, canvas);
}

/// 覆盖率表必须含大量「部分覆盖」子像素（软边来源），且真彩合成把软边向画布渐隐。
#[test]
fn splash_composite_soft_edges_exist() {
    let partial = splash_coverage::RICH_COVERAGE
        .iter()
        .filter(|&&c| c > 10 && c < 245)
        .count();
    assert!(
        partial * 100 / splash_coverage::RICH_COVERAGE.len() >= 15,
        "软边子像素过少：{partial} / {}",
        splash_coverage::RICH_COVERAGE.len()
    );
    // 取一个真实的半覆盖值做合成，结果必须既不是画布色也不是满色。
    let canvas = [18u8, 16, 14];
    let grad = splash_gradient_rgb(0.5);
    let mid = splash_composite_color(grad, 128, true);
    assert_ne!(mid, canvas);
    assert_ne!(mid, grad);
}

// ── 渲染栅格：行数 / 行宽不越界 ─────────────────────────────────────────────

/// 品牌画每档的字符行数必须 = 子像素行/2；每行品牌格数必须 = 列数（不越界）。
#[test]
fn splash_banner_grid_matches_tier() {
    let mut state = TuiState::default();
    state.model = "m".to_string();
    state.current_provider_id = "p".to_string();
    state.workspace = std::path::PathBuf::from("/w");

    for tier in [SplashTier::Rich, SplashTier::Compact] {
        let cols = tier.cols();
        let rows = tier.rows();
        let lines = pony_banner_lines(&state, tier, true);
        assert_eq!(lines.len(), rows / 2, "字符行数必须 = 子像素行/2");

        for (r, line) in lines.iter().enumerate() {
            // spans[0] 是前导空格，其后恰好 cols 个品牌格（"▀" 或空白 " "），再接信息列。
            let cells = line
                .spans
                .iter()
                .skip(1)
                .take_while(|span| {
                    let c = span.content.as_ref();
                    c == "▀" || c == " "
                })
                .count();
            assert_eq!(cells, cols, "第 {r} 行品牌格数必须 = 列数 {cols}");
            // 行宽不超过「前导 1 + 列 cols + 分隔 4 + 信息列」的合理上界（防跑飞）。
            assert!(
                line.width() <= 1 + cols + 4 + 48,
                "第 {r} 行行宽越界: {}",
                line.width()
            );
        }
    }
}

// ── 生成脚本漂移门禁（--check 幂等）──────────────────────────────────────

/// `scripts/generate-splash-logo.py --check` 必须对已提交产物返回 0（无漂移）。
/// 无 python3 / numpy / PIL 的环境跳过（脚本本身另有独立 `--check` 入口供 CI 调用）。
#[test]
fn generate_script_check_is_idempotent() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let script = format!("{root}/scripts/generate-splash-logo.py");
    if !std::path::Path::new(&script).exists() {
        eprintln!("skip: 未找到生成脚本 {script}");
        return;
    }
    let deps_ok = std::process::Command::new("python3")
        .args(["-c", "import numpy, PIL"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !deps_ok {
        eprintln!("skip: python3 或 numpy/PIL 不可用，跳过生成脚本漂移校验");
        return;
    }
    let out = std::process::Command::new("python3")
        .arg(&script)
        .arg("--check")
        .current_dir(root)
        .output()
        .expect("运行生成脚本 --check");
    assert!(
        out.status.success(),
        "splash 生成产物漂移，请运行 python3 scripts/generate-splash-logo.py：\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
