//! 初始化页品牌标（原 `tui.rs` 内联实现，机械外移）。
//! 三件事：轮廓与配色数据（`splash_coverage.rs` 生成物：覆盖率表 + 母版真色表）、
//! 覆盖率抗锯齿、终端图形协议判定（纯环境判定，**不触碰 stdin**）。全部无状态、可独立单测。

use super::*;

#[path = "splash_coverage.rs"]
mod splash_coverage;

/// 画布底色（必须等于 `theme::CANVAS`；既是 AA 合成的底色，也是位图底色）。
const SPLASH_CANVAS_RGB: [u8; 3] = [18, 16, 14];

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

/// 覆盖率 → 绘制色：真彩把母版真色 `ink` 按 `cov` 向画布色混合（边缘向画布渐隐＝终端里的抗锯齿）；
/// 非真彩退化为硬阈值（≥50% 才上色），保证不劣于旧版。
fn splash_composite_color(ink: [u8; 3], cov: u8, truecolor: bool) -> [u8; 3] {
    if !truecolor {
        return if cov >= 128 { ink } else { SPLASH_CANVAS_RGB };
    }
    let a = cov as f32 / 255.0;
    let mix = |c: u8, base: u8| -> u8 { (base as f32 + (c as f32 - base as f32) * a).round() as u8 };
    [
        mix(ink[0], SPLASH_CANVAS_RGB[0]),
        mix(ink[1], SPLASH_CANVAS_RGB[1]),
        mix(ink[2], SPLASH_CANVAS_RGB[2]),
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
    /// 每个子像素的母版真色（RGB 三连字节，行优先，与 `coverage()` 同序）。
    fn master_rgb(self) -> &'static [u8] {
        match self {
            SplashTier::Rich => &splash_coverage::RICH_RGB,
            SplashTier::Compact => &splash_coverage::COMPACT_RGB,
        }
    }
}

/// 图形协议品牌标的字符占位（与 Rich 档字符画同尺寸：RICH_COLS 列 × RICH_ROWS/2 行）。
pub const SPLASH_IMAGE_COLS: u16 = splash_coverage::RICH_COLS as u16;
pub const SPLASH_IMAGE_ROWS: u16 = (splash_coverage::RICH_ROWS / 2) as u16;
/// 品牌标位图（由 `scripts/generate-splash-logo.py` 生成，画布底色不透明，
/// 用以完整覆盖其下方的字符画占位）。
pub const SPLASH_LOGO_PNG: &[u8] = include_bytes!("../../../assets/splash-logo.png");

/// 初始化页品牌标的位图路径：sixel / kitty / iTerm2 / halfblocks 四种协议都走同一张位图。
/// halfblocks 是纯 Rust 的 24bit 前后景半块渲染，与 kitty/sixel 共享同一像素源，
/// 因此在没有图形协议的普通终端里也能拿到母版真色观感（ADR 0098）。
pub fn init_splash_image() -> Option<SplashImage> {
    let mut picker = splash_picker()?;
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

/// `PONY_TUI_SPLASH_IMAGE_PROTOCOL` 的纯映射（零 I/O，可独立单测）：
/// `Some(Some(protocol))` 强制协议；`Some(None)` 关闭位图路径；`None` 回落环境判定。
/// 注意 `halfblocks` 是**强制半块位图协议**（母版真色渲染），不是关闭位图。
fn splash_protocol_override(raw: Option<&str>) -> Option<Option<ProtocolType>> {
    match raw {
        Some("off") => Some(None),
        Some("halfblocks") => Some(Some(ProtocolType::Halfblocks)),
        Some("kitty") => Some(Some(ProtocolType::Kitty)),
        Some("sixel") => Some(Some(ProtocolType::Sixel)),
        Some("iterm2") => Some(Some(ProtocolType::Iterm2)),
        _ => None,
    }
}

fn splash_picker() -> Option<Picker> {
    // auto（默认）走上面的无 I/O 环境判定；其余见 `splash_protocol_override`。
    let override_protocol =
        splash_protocol_override(std::env::var("PONY_TUI_SPLASH_IMAGE_PROTOCOL").ok().as_deref());
    let protocol = match override_protocol {
        Some(protocol) => protocol?,
        None => splash_protocol_for_env(&SplashEnv::from_process())?,
    };
    Some(splash_picker_for_protocol(protocol))
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

/// 彩色 init 头部品牌画：轮廓与配色都来自官方母版的派生表（见 `splash_coverage.rs`）——
/// 覆盖率表给每个子像素的抗锯齿权重，真色表给母版在该子像素的平均 RGB。每格固定写半块字符
/// `▀`，上半格取 `fg`、下半格取 `bg`，两者都是 `mix(CANVAS, master_rgb, coverage)`：
/// 覆盖率让边缘向画布色渐隐，这是终端里能做到的真实抗锯齿（真彩门禁失效时退化为硬阈值）。
/// 右侧对齐品牌控制台信息列。`draw_logo = false` 时只保留占位空白（由位图覆盖该矩形）。
fn pony_banner_lines(state: &TuiState, tier: SplashTier, draw_logo: bool) -> Vec<Line<'static>> {
    let cols = tier.cols();
    let rows = tier.rows();
    let coverage = tier.coverage();
    let master_rgb = tier.master_rgb();
    let line_count = rows / 2;
    let truecolor = splash_truecolor_enabled();
    let canvas = ratatui::style::Color::Rgb(
        SPLASH_CANVAS_RGB[0],
        SPLASH_CANVAS_RGB[1],
        SPLASH_CANVAS_RGB[2],
    );

    // 母版真色 + 覆盖率合成：边缘按覆盖率向画布色渐隐。
    let cell_color = |x: usize, y: usize| -> ratatui::style::Color {
        let idx = y * cols + x;
        let cov = coverage.get(idx).copied().unwrap_or(0);
        let base = idx * 3;
        let rgb = match (master_rgb.get(base), master_rgb.get(base + 1), master_rgb.get(base + 2)) {
            (Some(&r), Some(&g), Some(&b)) => splash_composite_color([r, g, b], cov, truecolor),
            _ => SPLASH_CANVAS_RGB,
        };
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
/// 高度阶梯 → 宽度降档 → 位图仅在 Rich 档且宽度足够时铺。
///
/// 尺寸阶梯（`avail` 已把底部呼吸行加回；尺寸按 ADR 0105 收到原 1/3）：
///   富余 ≥ 28 行 → Rich（9 列 × 10 子像素 = 5 行品牌画）
///   紧凑 ≥ 24 行 → Compact（7 列 × 8 子像素 = 4 行品牌画）
///   更矮 → 隐藏品牌画，只留定位语 + 命令入口。
/// 阈值沿用 ADR 0098 的 28 / 24：ADR 0105 只缩品牌画尺寸，不动降级阶梯。
/// 构图（含引导块）= 1 空行 + 品牌画 + 1 空行 + 定位语 + 1 空行 + 引导块 8 行；
/// Rich 共 17 行、Compact 共 16 行，在两档阈值内都有充足余量。
/// 返回值第二项为「是否走位图」（此时 `lines` 是等宽占位空白）。位图矩形固定 9×5，
/// 与 Compact 的 7×4 占位不重合，故仅在 Rich 档启用。
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
