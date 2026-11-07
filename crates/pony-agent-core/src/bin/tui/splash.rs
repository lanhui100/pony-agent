//! 初始化页品牌标（原 `tui.rs` 内联实现，机械外移）。
//! 三件事：轮廓数据（`splash_coverage.rs` 生成物）、OKLab 渐变 + 覆盖率抗锯齿、
//! 终端图形协议判定（纯环境判定，**不触碰 stdin**）。全部无状态、可独立单测。

use super::*;

#[path = "splash_coverage.rs"]
mod splash_coverage;

/// sRGB 8bit 分量 → 线性光分量（IEC 61966-2-1 传输函数）。
fn srgb_channel_to_linear(channel: u8) -> f32 {
    let v = channel as f32 / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// 线性光分量 → sRGB 8bit 分量（夹取到 [0,255]，越界插值不会写坏终端色）。
fn linear_channel_to_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
}

/// sRGB → OKLab（Björn Ottosson, 2020）：感知均匀空间，插值不经过发灰的暗带。
fn srgb_to_oklab(rgb: [u8; 3]) -> [f32; 3] {
    let r = srgb_channel_to_linear(rgb[0]);
    let g = srgb_channel_to_linear(rgb[1]);
    let b = srgb_channel_to_linear(rgb[2]);
    let l = 0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b;
    let m = 0.211_903_5 * r + 0.680_699_54 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_8 * s_,
    ]
}

/// OKLab → sRGB（越界分量由 [`linear_channel_to_srgb`] 夹取）。
fn oklab_to_srgb(lab: [f32; 3]) -> [u8; 3] {
    let [l, a, b] = lab;
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        linear_channel_to_srgb(4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_93 * s3),
        linear_channel_to_srgb(-1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3),
        linear_channel_to_srgb(-0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3),
    ]
}

/// 启动页品牌标渐变（C2 克制暖色，与 `mod theme`「单色为骨 + 单一强调色」一致）：
/// 暖金 → 赤陶 → 紫。色标与 `scripts/generate-splash-logo.py` 的 `STOPS` 严格同源，禁止手改。
const SPLASH_GRADIENT_STOPS: [(f32, [u8; 3]); 3] = [
    (0.00, [255, 196, 120]), // #ffc478 暖金高光
    (0.45, [233, 110, 84]),  // #e96e54 赤陶
    (1.00, [150, 92, 220]),  // #965cdc 紫
];

/// 画布底色（必须等于 `theme::CANVAS`；既是 AA 合成的底色，也是位图底色）。
const SPLASH_CANVAS_RGB: [u8; 3] = [18, 16, 14];

/// 主对角线渐变采样：`t` 已归一化到掩码实际覆盖的 [0,1]。
/// 段内用 smoothstep 缓动 + OKLab 线性插值，色标接缝处一阶连续，不会出现硬色带。
fn splash_gradient_rgb(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut idx = 0usize;
    while idx + 2 < SPLASH_GRADIENT_STOPS.len() && t > SPLASH_GRADIENT_STOPS[idx + 1].0 {
        idx += 1;
    }
    let (t0, c0) = SPLASH_GRADIENT_STOPS[idx];
    let (t1, c1) = SPLASH_GRADIENT_STOPS[idx + 1];
    let k = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
    let k = k * k * (3.0 - 2.0 * k);
    let a = srgb_to_oklab(c0);
    let b = srgb_to_oklab(c1);
    oklab_to_srgb([
        a[0] + (b[0] - a[0]) * k,
        a[1] + (b[1] - a[1]) * k,
        a[2] + (b[2] - a[2]) * k,
    ])
}

/// 标记 bbox 的 45° 对角投影：`0.4·(x/(cols-1)) + 0.6·(y/(rows-1))`。
/// `(0,0)` → 0（暖金高光端），`(cols-1, rows-1)` → 1（紫收尾端），与生成脚本同口径。
fn splash_diagonal_t(x: usize, y: usize, cols: usize, rows: usize) -> f32 {
    let fx = x as f32 / (cols.max(2) - 1) as f32;
    let fy = y as f32 / (rows.max(2) - 1) as f32;
    (0.4 * fx + 0.6 * fy).clamp(0.0, 1.0)
}

/// 真彩门禁：`COLORTERM` 含 `truecolor` / `24bit` 才启用 alpha 抗锯齿（纯函数、零 I/O）。
fn colorterm_supports_truecolor(raw: Option<&str>) -> bool {
    raw.map(|value| {
        let value = value.to_ascii_lowercase();
        value.contains("truecolor") || value.contains("24bit")
    })
    .unwrap_or(false)
}

/// 读取进程 `COLORTERM` 判定 AA 是否可用（只读环境变量，不触碰 stdio）。
fn splash_truecolor_enabled() -> bool {
    colorterm_supports_truecolor(std::env::var("COLORTERM").ok().as_deref())
}

/// 覆盖率 → 绘制色：真彩把 `grad` 按 `cov` 向画布色混合（边缘向画布渐隐＝终端里的抗锯齿）；
/// 非真彩退化为硬阈值（≥50% 才上色），保证不劣于旧版。
fn splash_composite_color(grad: [u8; 3], cov: u8, truecolor: bool) -> [u8; 3] {
    if !truecolor {
        return if cov >= 128 { grad } else { SPLASH_CANVAS_RGB };
    }
    let a = cov as f32 / 255.0;
    let mix = |c: u8, base: u8| -> u8 { (base as f32 + (c as f32 - base as f32) * a).round() as u8 };
    [
        mix(grad[0], SPLASH_CANVAS_RGB[0]),
        mix(grad[1], SPLASH_CANVAS_RGB[1]),
        mix(grad[2], SPLASH_CANVAS_RGB[2]),
    ]
}

/// 品牌标字符栅格档位：尺寸阶梯随可用高度自适应（富余 Rich / 紧凑 Compact / 更矮隐藏）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplashTier {
    Rich,
    Compact,
}

impl SplashTier {
    fn cols(self) -> usize {
        match self {
            SplashTier::Rich => splash_coverage::RICH_COLS,
            SplashTier::Compact => splash_coverage::COMPACT_COLS,
        }
    }
    fn rows(self) -> usize {
        match self {
            SplashTier::Rich => splash_coverage::RICH_ROWS,
            SplashTier::Compact => splash_coverage::COMPACT_ROWS,
        }
    }
    fn coverage(self) -> &'static [u8] {
        match self {
            SplashTier::Rich => &splash_coverage::RICH_COVERAGE,
            SplashTier::Compact => &splash_coverage::COMPACT_COVERAGE,
        }
    }
}

/// 图形协议品牌标的字符占位（与 Rich 档字符画同尺寸：RICH_COLS 列 × RICH_ROWS/2 行）。
pub const SPLASH_IMAGE_COLS: u16 = splash_coverage::RICH_COLS as u16;
pub const SPLASH_IMAGE_ROWS: u16 = (splash_coverage::RICH_ROWS / 2) as u16;
/// 品牌标位图（由 `scripts/generate-splash-logo.py` 生成，画布底色不透明，
/// 用以完整覆盖其下方的字符画占位）。
pub const SPLASH_LOGO_PNG: &[u8] = include_bytes!("../../../assets/splash-logo.png");

/// 初始化页品牌标的图形协议路径：仅在终端支持 sixel / kitty / iTerm2 时构建。
pub fn init_splash_image() -> Option<SplashImage> {
    let mut picker = splash_picker()?;
    if picker.protocol_type() == ProtocolType::Halfblocks {
        // halfblocks 即自绘半块字符，字符画版本的边缘与配色更可控，保留现路径。
        return None;
    }
    // 透明像素按画布底色合成，位图矩形与周围 Canvas 无缝衔接。
    picker.set_background_color(image::Rgba([
        SPLASH_CANVAS_RGB[0],
        SPLASH_CANVAS_RGB[1],
        SPLASH_CANVAS_RGB[2],
        255,
    ]));
    let image = image::load_from_memory(SPLASH_LOGO_PNG).ok()?;
    Some(SplashImage {
        protocol: picker.new_resize_protocol(image),
        picker,
        cols: SPLASH_IMAGE_COLS,
        rows: SPLASH_IMAGE_ROWS,
    })
}

/// 启动期图形协议判定的输入快照：只含环境变量与终端拓扑，**不触碰 stdin**。
///
/// 缺陷背景：`ratatui-image` 的 stdio 能力查询会另起线程对 stdin 做
/// 阻塞 `read`（1s 超时后线程不回收、继续吃后续按键），并自行调用 `disable_raw_mode()`
/// 把上层刚建立的 raw mode 关掉。结果是「启动期敲的字被吞」+「事件循环被降级到行缓冲」。
/// 品牌标是装饰性内容，不值得拿输入可靠性去换，因此这里改为纯环境判定（零 I/O）；
/// 回归守卫见 `splash_gradient_tests::splash_probe_never_reads_stdin`。
#[derive(Debug, Default, Clone)]
pub struct SplashEnv {
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub lc_terminal: Option<String>,
    pub kitty_window_id: Option<String>,
    /// 命中的复用器 / pty 代理层环境变量名（见 [`terminal_multiplexer`]）。
    pub multiplexer: Option<String>,
}

impl SplashEnv {
    /// 读取真实进程环境（纯读取，无 I/O 副作用、不会消费任何输入）。
    pub fn from_process() -> Self {
        fn get(key: &str) -> Option<String> {
            std::env::var(key)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        }
        Self {
            term: get("TERM"),
            term_program: get("TERM_PROGRAM"),
            lc_terminal: get("LC_TERMINAL"),
            kitty_window_id: get("KITTY_WINDOW_ID"),
            multiplexer: terminal_multiplexer(),
        }
    }
}

/// **唯一决策点**：启动期品牌标走哪种图形协议；`None` ⇒ 回退自绘字符画（fail-closed）。
///
/// 判定顺序即风险排序：kitty 的 APC 序列在不支持的终端上会被静默忽略（最坏情况只是
/// 保留下方字符画），而 sixel 一旦误判会往屏幕喷像素乱码，因此 kitty 优先命中、
/// sixel 只认明确声明支持的终端。
pub fn splash_protocol_for_env(env: &SplashEnv) -> Option<ProtocolType> {
    // 复用器 / pty 代理层的图形直通依赖额外开关（如 tmux 的 allow-passthrough）且探测易误判；
    // 用户可用 PONY_TUI_SPLASH_IMAGE_PROTOCOL=kitty|sixel|iterm2 显式强制开启。
    if env.multiplexer.is_some() {
        return None;
    }
    let lower = |value: &Option<String>| value.as_deref().unwrap_or_default().to_ascii_lowercase();
    let term = lower(&env.term);
    let term_program = lower(&env.term_program);
    let lc_terminal = lower(&env.lc_terminal);

    let kitty = env.kitty_window_id.is_some()
        || term.contains("kitty")
        // Ghostty 与 WezTerm 均实现 kitty 图形协议（不兼容时 APC 被忽略，风险可控）。
        || term_program.contains("ghostty")
        || term_program.contains("wezterm");
    if kitty {
        return Some(ProtocolType::Kitty);
    }
    if term_program.contains("iterm") || lc_terminal.contains("iterm") {
        return Some(ProtocolType::Iterm2);
    }
    let sixel = term.contains("sixel")
        || ["foot", "contour", "mlterm", "yaft", "mintty"]
            .iter()
            .any(|name| term.contains(name));
    if sixel {
        return Some(ProtocolType::Sixel);
    }
    None
}

fn splash_picker() -> Option<Picker> {
    // PONY_TUI_SPLASH_IMAGE_PROTOCOL 为手动/测试覆盖入口：
    // auto（默认）走上面的无 I/O 环境判定；kitty/sixel/iterm2 强制协议；off 禁用图形路径。
    match std::env::var("PONY_TUI_SPLASH_IMAGE_PROTOCOL").as_deref() {
        Ok("off") | Ok("halfblocks") => None,
        Ok("kitty") => Some(splash_picker_for_protocol(ProtocolType::Kitty)),
        Ok("sixel") => Some(splash_picker_for_protocol(ProtocolType::Sixel)),
        Ok("iterm2") => Some(splash_picker_for_protocol(ProtocolType::Iterm2)),
        _ => Some(splash_picker_for_protocol(splash_protocol_for_env(
            &SplashEnv::from_process(),
        )?)),
    }
}

/// 由协议 + 字符格像素尺寸构造 picker：不查询终端，拿不到回包也不会吞输入。
fn splash_picker_for_protocol(protocol: ProtocolType) -> Picker {
    let mut picker = Picker::from_fontsize(splash_font_px());
    picker.set_protocol_type(protocol);
    picker
}

/// 字符格像素尺寸：env 覆盖 > `TIOCGWINSZ` 像素字段 > 8×16 默认值。
fn splash_font_px() -> (u16, u16) {
    let from_env = std::env::var("PONY_TUI_SPLASH_FONT_PX").ok().and_then(|raw| {
        let (width, height) = raw.split_once('x')?;
        Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
    });
    from_env.or_else(tty_cell_pixel_size).unwrap_or((8, 16))
}

/// 从终端窗口尺寸反推字符格像素大小（纯 ioctl，不读输入）。
/// 多数终端不上报 `ws_xpixel/ws_ypixel`，此时返回 `None` 由调用方回落默认值。
#[cfg(unix)]
fn tty_cell_pixel_size() -> Option<(u16, u16)> {
    use std::os::unix::io::AsRawFd;
    let stdout = std::io::stdout();
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(stdout.as_raw_fd(), libc::TIOCGWINSZ, &mut size) };
    if rc != 0 {
        return None;
    }
    if size.ws_xpixel == 0 || size.ws_ypixel == 0 || size.ws_col == 0 || size.ws_row == 0 {
        return None;
    }
    let cell = (size.ws_xpixel / size.ws_col, size.ws_ypixel / size.ws_row);
    (cell.0 > 0 && cell.1 > 0).then_some(cell)
}

#[cfg(not(unix))]
fn tty_cell_pixel_size() -> Option<(u16, u16)> {
    None
}

/// 彩色 init 头部品牌画：轮廓来自官方母版的覆盖率重采样（见 `splash_coverage.rs`），
/// 每格固定写半块字符 `▀`，上半格取 `fg`、下半格取 `bg`，两者都是
/// `mix(CANVAS, gradient(t), coverage)` —— 覆盖率让边缘向画布色渐隐，这是终端里能做到的
/// 真实抗锯齿（真彩门禁失效时退化为硬阈值，观感不劣于旧版）。右侧对齐品牌控制台信息列。
/// `draw_logo = false` 时只保留占位空白（由图形协议位图覆盖该矩形）。
fn pony_banner_lines(state: &TuiState, tier: SplashTier, draw_logo: bool) -> Vec<Line<'static>> {
    let cols = tier.cols();
    let rows = tier.rows();
    let coverage = tier.coverage();
    let line_count = rows / 2;
    let truecolor = splash_truecolor_enabled();
    let canvas = ratatui::style::Color::Rgb(
        SPLASH_CANVAS_RGB[0],
        SPLASH_CANVAS_RGB[1],
        SPLASH_CANVAS_RGB[2],
    );

    // 覆盖率合成：`t` 取标记 bbox 的 45° 轴投影，边缘按覆盖率向画布色渐隐。
    let cell_color = |x: usize, y: usize| -> ratatui::style::Color {
        let grad = splash_gradient_rgb(splash_diagonal_t(x, y, cols, rows));
        let cov = coverage.get(y * cols + x).copied().unwrap_or(0);
        let rgb = splash_composite_color(grad, cov, truecolor);
        ratatui::style::Color::Rgb(rgb[0], rgb[1], rgb[2])
    };

    let info = [
        Line::from(vec![
            Span::styled(
                "Pony",
                Style::default()
                    .fg(ratatui::style::Color::Rgb(249, 115, 22))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " Agent",
                Style::default()
                    .fg(theme::INK)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(theme::MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("模型 · ", Style::default().fg(theme::MUTED)),
            Span::styled(
                state.model.clone(),
                Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({})", state.current_provider_id),
                Style::default().fg(theme::GHOST),
            ),
        ]),
        Line::from(vec![
            Span::styled("工作区 · ", Style::default().fg(theme::MUTED)),
            Span::styled(
                state.workspace.display().to_string(),
                Style::default().fg(theme::MUTED),
            ),
        ]),
    ];

    let mut out: Vec<Line<'static>> = Vec::new();
    // 信息列垂直居中于品牌画的字符行（3 行信息）。
    let info_top = line_count.saturating_sub(info.len()) / 2;
    for line in 0..line_count {
        let (top, bottom) = (line * 2, line * 2 + 1);
        let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
        if draw_logo {
            for x in 0..cols {
                let up = cell_color(x, top);
                let down = cell_color(x, bottom);
                if up == canvas && down == canvas {
                    // 上下半格都落在画布上：留空格，避免给纯画布区域加多余样式。
                    spans.push(Span::raw(" "));
                } else {
                    spans.push(Span::styled("▀", Style::default().fg(up).bg(down)));
                }
            }
        } else {
            // 图形协议路径：品牌标矩形由位图覆盖，这里只铺占位空白保持信息列对齐。
            spans.push(Span::raw(" ".repeat(cols)));
        }
        if let Some(info_line) = line.checked_sub(info_top).and_then(|i| info.get(i)) {
            spans.push(Span::raw("    "));
            spans.extend(info_line.spans.clone());
        }
        out.push(Line::from(spans));
    }
    out
}

/// 空态品牌画选档（`None` = 终端太矮，隐藏品牌画）：
/// 高度阶梯 → 宽度降档 → 图形协议位图仅在 Rich 档且宽度足够时铺。
///
/// 尺寸阶梯（`avail` 已把底部呼吸行加回）：
///   富余 ≥ 28 行 → Rich（27 列 × 30 子像素 = 15 行品牌画）
///   紧凑 ≥ 24 行 → Compact（20 列 × 22 子像素 = 11 行品牌画）
///   更矮 → 隐藏品牌画，只留定位语 + 命令入口。
/// 构图（含引导块）= 1 空行 + 品牌画 + 1 空行 + 定位语 + 1 空行 + 引导块 8 行；
/// Rich 共 27 行、Compact 共 23 行，各留 1 行余量，故阈值分别取 28 / 24。
/// 返回值第二项为「是否走图形协议位图」（此时 `lines` 是等宽占位空白）。
pub fn splash_banner(
    state: &TuiState,
    width: usize,
    chat_h: u16,
) -> Option<(Vec<Line<'static>>, bool)> {
    // C8 修订（L3-B F4）：`chat_h` 已扣除底部呼吸行，阈值判定要把它加回来，
    // 否则空态可用高度凭空少 1 行，height=31/35 时品牌画会比改动前消失一格。
    let avail = chat_h as usize + CHAT_BOTTOM_GAP as usize;
    const SPLASH_RICH_MIN_LINES: usize = 28;
    const SPLASH_COMPACT_MIN_LINES: usize = 24;
    if avail < SPLASH_COMPACT_MIN_LINES {
        return None;
    }
    let mut tier = if avail >= SPLASH_RICH_MIN_LINES {
        SplashTier::Rich
    } else {
        SplashTier::Compact
    };
    // 先按占位版量宽：宽度不足先降一档（Rich → Compact）。
    let fits = |lines: &[Line<'static>]| lines.iter().all(|line| line.width() <= width);
    if tier == SplashTier::Rich && !fits(&pony_banner_lines(state, tier, false)) {
        tier = SplashTier::Compact;
    }
    let placeholder = pony_banner_lines(state, tier, false);
    let banner_fits = fits(&placeholder);
    let use_image = state.splash_image.is_some() && banner_fits && tier == SplashTier::Rich;
    let lines = if use_image {
        placeholder
    } else {
        pony_banner_lines(state, tier, true)
    };
    Some((lines, use_image))
}

pub fn ascii_logo() -> Vec<Line<'static>> {
    // 空态由 ui() 组合：splash_banner(state) + 引导块；
    // 此函数保留独立可测的引导块。所有文案缩进 2 列，与消息正文共享文本列。
    let mut lines = vec![
        Line::from(Span::styled("  描述一个任务开始，或输入命令：", ink_soft())),
        blank(),
    ];
    // 直接复用命令表，保证空态提示与斜杠菜单永不漂移。
    for (cmd, desc) in AVAILABLE_COMMANDS.iter().take(4) {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(pad_display(cmd, 11), ink_soft()),
            Span::styled(desc.to_string(), muted()),
        ]));
    }
    lines.push(blank());
    lines.push(Line::from(Span::styled("  键入 / 查看全部命令", faint())));
    lines
}

#[cfg(test)]
#[path = "tests/splash_gradient_tests.rs"]
mod splash_gradient_tests;
