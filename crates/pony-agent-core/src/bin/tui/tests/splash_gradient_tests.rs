//! 原 `tui.rs` 内联测试模块 `mod splash_gradient_tests`（Step1 机械外移）。
//! ADR 0098 后扩为「生成物来源/尺寸 + 母版真色表 + AA 合成 + 真彩门禁 + 尺寸阶梯」一组门禁。

use super::*;

/// 取覆盖率表里某个子像素的母版真色（越界返回画布色，便于断言不 panic）。
fn master_rgb_at(cols: usize, rows: usize, rgb: &[u8], x: usize, y: usize) -> [u8; 3] {
    assert!(x < cols && y < rows, "子像素坐标越界");
    let base = (y * cols + x) * 3;
    [rgb[base], rgb[base + 1], rgb[base + 2]]
}

/// 覆盖率表必须与生成脚本声明的档位尺寸严格一致，且带「禁止手改 / 来源 master」头注释。
/// 这是「单一真值源 = 官方母版重采样」的编译期 + 运行期双重锚点。
#[test]
fn splash_coverage_tables_match_declared_dims() {
    assert_eq!(splash_coverage::RICH_COLS, 9, "Rich 档列数");
    assert_eq!(splash_coverage::RICH_ROWS, 10, "Rich 档子像素行数");
    assert_eq!(splash_coverage::COMPACT_COLS, 7, "Compact 档列数");
    assert_eq!(splash_coverage::COMPACT_ROWS, 8, "Compact 档子像素行数");

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
    // 真色表与覆盖率表同序、同基数：每子像素 3 字节。
    assert_eq!(
        splash_coverage::RICH_RGB.len(),
        splash_coverage::RICH_COLS * splash_coverage::RICH_ROWS * 3,
        "Rich 真色表长度必须 = 子像素数 × 3"
    );
    assert_eq!(
        splash_coverage::COMPACT_RGB.len(),
        splash_coverage::COMPACT_COLS * splash_coverage::COMPACT_ROWS * 3,
        "Compact 真色表长度必须 = 子像素数 × 3"
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
    assert!(
        source.contains("RICH_RGB") && source.contains("COMPACT_RGB"),
        "真色表必须同时导出 Rich / Compact 两档"
    );
}

#[test]
fn splash_logo_asset_decodes_for_image_protocol() {
    // 位图由同一 master 生成，铺满 Rich 档字符矩形（9×5 格 × 8×16 像素 × 4 超采样）。
    let img = image::load_from_memory(SPLASH_LOGO_PNG).expect("内嵌品牌标必须可解码");
    assert_eq!((img.width(), img.height()), (288, 320));
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

// ── 母版真色表：配色即母版，不再走自造渐变 ─────────────────────────────────

/// 真色表必须真的携带母版的彩色信息（而不是被画布色/单色渐变抹平）：
/// 覆盖率 ≥50% 的子像素里，绝大多数必须有明显彩度，且色相分布同时覆盖紫系与暖金系。
#[test]
fn splash_master_rgb_table_keeps_master_colors() {
    let cols = splash_coverage::RICH_COLS;
    let rows = splash_coverage::RICH_ROWS;
    let mut covered = 0usize;
    let mut chromatic = 0usize;
    let mut purple = 0usize;
    let mut warm = 0usize;
    let mut chroma_sum = 0usize;
    let mut max_chroma = 0u8;
    for y in 0..rows {
        for x in 0..cols {
            let cov = splash_coverage::RICH_COVERAGE[y * cols + x];
            if cov < 128 {
                continue;
            }
            covered += 1;
            let rgb = master_rgb_at(cols, rows, &splash_coverage::RICH_RGB, x, y);
            let chroma = rgb.iter().max().unwrap() - rgb.iter().min().unwrap();
            chroma_sum += chroma as usize;
            max_chroma = max_chroma.max(chroma);
            if chroma > 40 {
                chromatic += 1;
            }
            if rgb[2] > rgb[0] {
                purple += 1;
            }
            if rgb[0] > rgb[2] && rgb[1] > rgb[2] {
                warm += 1;
            }
        }
    }
    assert!(
        covered * 100 / (cols * rows) >= 20,
        "Rich 档有效子像素过少：{covered} / {}（ADR 0105 缩小后按占比判定）",
        cols * rows
    );
    let ratio = chromatic as f64 / covered as f64;
    assert!(
        ratio >= 0.9,
        "母版真色被抹平：彩度 >40 的比例仅 {ratio:.3}（{chromatic}/{covered}）"
    );
    let mean_chroma = chroma_sum as f64 / covered as f64;
    assert!(
        mean_chroma >= 90.0,
        "母版真色平均彩度偏低：{mean_chroma:.1}（期望 ≥90，实测母版 124.7）"
    );
    assert!(max_chroma >= 150, "母版真色最高彩度偏低：{max_chroma}");
    // 母版是「紫红丝带 + 暖金高光」，两个色系都必须占到可见份额。
    let purple_ratio = purple as f64 / covered as f64;
    let warm_ratio = warm as f64 / covered as f64;
    assert!(
        purple_ratio >= 0.15,
        "紫系占比过低：{purple_ratio:.3}（母版实测 ≈0.30）"
    );
    assert!(
        warm_ratio >= 0.15,
        "暖金系占比过低：{warm_ratio:.3}（母版实测 ≈0.31）"
    );
}

/// 真色表必须与覆盖率表同源同序：完全未覆盖的子像素取画布色（不留脏数据），
/// 且整表不得出现「覆盖率 0 却带彩度」的错位。
#[test]
fn splash_master_rgb_aligns_with_coverage() {
    for (cols, rows, cov, rgb) in [
        (
            splash_coverage::RICH_COLS,
            splash_coverage::RICH_ROWS,
            &splash_coverage::RICH_COVERAGE[..],
            &splash_coverage::RICH_RGB[..],
        ),
        (
            splash_coverage::COMPACT_COLS,
            splash_coverage::COMPACT_ROWS,
            &splash_coverage::COMPACT_COVERAGE[..],
            &splash_coverage::COMPACT_RGB[..],
        ),
    ] {
        let mut empty = 0usize;
        for y in 0..rows {
            for x in 0..cols {
                let idx = y * cols + x;
                let color = master_rgb_at(cols, rows, rgb, x, y);
                if cov[idx] == 0 {
                    empty += 1;
                    let chroma = color.iter().max().unwrap() - color.iter().min().unwrap();
                    assert!(
                        chroma <= 4,
                        "覆盖率 0 的子像素 ({x},{y}) 却带彩度 {chroma}: {color:?}"
                    );
                }
            }
        }
        assert!(
            empty * 100 / cov.len() >= 30,
            "轮廓外空白过少（覆盖率 0 仅 {empty}/{}），真色表可能错位",
            cov.len()
        );
    }
}

/// 两个档位的真色表都必须与画布色有足够区分：合成后品牌画不能被画布吞掉。
#[test]
fn splash_master_rgb_differs_from_canvas() {
    let canvas = SPLASH_CANVAS_RGB;
    for (cols, rows, cov, rgb) in [
        (
            splash_coverage::RICH_COLS,
            splash_coverage::RICH_ROWS,
            &splash_coverage::RICH_COVERAGE[..],
            &splash_coverage::RICH_RGB[..],
        ),
        (
            splash_coverage::COMPACT_COLS,
            splash_coverage::COMPACT_ROWS,
            &splash_coverage::COMPACT_COVERAGE[..],
            &splash_coverage::COMPACT_RGB[..],
        ),
    ] {
        let mut distinct = 0usize;
        let mut covered = 0usize;
        for y in 0..rows {
            for x in 0..cols {
                if cov[y * cols + x] < 128 {
                    continue;
                }
                covered += 1;
                let color = master_rgb_at(cols, rows, rgb, x, y);
                let delta: i32 = color
                    .iter()
                    .zip(canvas.iter())
                    .map(|(a, b)| (*a as i32 - *b as i32).abs())
                    .sum();
                if delta > 30 {
                    distinct += 1;
                }
            }
        }
        assert!(
            distinct * 100 / covered >= 90,
            "与画布色几乎无差别的子像素过多（仅 {distinct}/{covered} 个有效色）"
        );
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
    // 取一个真实的半覆盖值 + 真实母版色做合成，结果必须既不是画布色也不是满色。
    let canvas = SPLASH_CANVAS_RGB;
    let (idx, cov) = splash_coverage::RICH_COVERAGE
        .iter()
        .enumerate()
        .find(|(_, &c)| c > 100 && c < 160)
        .map(|(i, &c)| (i, c))
        .expect("Rich 覆盖率表必须含半覆盖子像素");
    let ink = [
        splash_coverage::RICH_RGB[idx * 3],
        splash_coverage::RICH_RGB[idx * 3 + 1],
        splash_coverage::RICH_RGB[idx * 3 + 2],
    ];
    let mid = splash_composite_color(ink, cov, true);
    assert_ne!(mid, canvas, "软边合成塌成画布色");
    assert_ne!(mid, ink, "软边合成未向画布渐隐");
    for channel in 0..3 {
        let (m, i, c) = (mid[channel], ink[channel], canvas[channel]);
        let lo = i.min(c);
        let hi = i.max(c);
        assert!(
            m >= lo && m <= hi,
            "通道 {channel} 的软边值 {m} 未落在 [{lo},{hi}] 区间"
        );
    }
}

// ── 位图路径：四种协议同源，halfblocks 不再被排除 ──────────────────────────

/// `PONY_TUI_SPLASH_IMAGE_PROTOCOL` 必须把 `halfblocks` 映射为**启用半块位图协议**，
/// 只有 `off` 关闭位图路径（ADR 0098：母版真色 + 满宽构图，普通终端同样受益）。
#[test]
fn splash_protocol_override_keeps_halfblocks_bitmap() {
    assert_eq!(splash_protocol_override(Some("off")), Some(None));
    assert_eq!(
        splash_protocol_override(Some("halfblocks")),
        Some(Some(ProtocolType::Halfblocks))
    );
    assert_eq!(
        splash_protocol_override(Some("kitty")),
        Some(Some(ProtocolType::Kitty))
    );
    assert_eq!(
        splash_protocol_override(Some("sixel")),
        Some(Some(ProtocolType::Sixel))
    );
    assert_eq!(
        splash_protocol_override(Some("iterm2")),
        Some(Some(ProtocolType::Iterm2))
    );
    // auto / 未知值一律交给无 I/O 环境判定。
    assert_eq!(splash_protocol_override(None), None);
    assert_eq!(splash_protocol_override(Some("auto")), None);
    assert_eq!(splash_protocol_override(Some("")), None);
}

/// 位图构建不得再按协议类型提前放弃：halfblocks 与 kitty/sixel 共享同一张母版位图。
/// 断言的是源码形态（`init_splash_image` 内不再出现按协议早退的分支）。
#[test]
fn splash_image_init_has_no_protocol_early_return() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/tui/splash.rs");
    let source = std::fs::read_to_string(path).expect("读取 splash.rs 源码");
    let start = source.find("pub fn init_splash_image").expect("必须存在 init_splash_image");
    let body = &source[start..];
    let end = body.find("\n}\n").expect("init_splash_image 必须有结束大括号");
    let body = &body[..end];
    let forbidden = ["protocol_type() ==", "Halfblocks {"].concat();
    assert!(
        !body.contains(&forbidden),
        "init_splash_image 不得按协议早退（halfblocks 必须走位图）：命中 `{forbidden}`"
    );
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
