//! Design tokens（原 `tui.rs` 内联 `mod theme`，机械外移，内容零改动）。
//! 单一真值源：所有颜色只在此定义，禁止在渲染层写死 `Color::Rgb`。

use ratatui::style::Color;

pub const CANVAS: Color = Color::Rgb(18, 16, 14);
pub const SURFACE: Color = Color::Rgb(27, 25, 22);
pub const SURFACE_SEL: Color = Color::Rgb(40, 37, 32);
/// 代码块材质底色：加强深沉温暖的陶土黑底，对比鲜明同时与整套陶土暖黑画布自然呼应。
pub const CODE_FILL: Color = Color::Rgb(36, 25, 22);
pub const INK: Color = Color::Rgb(232, 229, 224);
pub const INK_SOFT: Color = Color::Rgb(203, 200, 195);
pub const MUTED: Color = Color::Rgb(162, 155, 146);
pub const FAINT: Color = Color::Rgb(124, 118, 110);
/// wave-39 v3 R10：指标行专用，比 FAINT 再暗一档（终端无法改字号，以低对比度表达「更小更弱」）。
pub const GHOST: Color = Color::Rgb(104, 98, 90);
pub const HAIRLINE: Color = Color::Rgb(52, 47, 41);
/// 强调色：更深沉醒目的陶土色（深赤陶色 Terracotta，温暖有质感）。
pub const ACCENT: Color = Color::Rgb(214, 114, 80);
/// 角色竖条等「结构性强调」用次级陶土色。
pub const ACCENT_DIM: Color = Color::Rgb(180, 92, 62);
/// 强化强调色：行内 **强调** 专用，明艳饱满的陶土砖红色，凸显视觉重点。
pub const EMPHASIS: Color = Color::Rgb(232, 128, 92);
/// 明亮陶土色（Bright Terracotta）：提问标题、重点问题专用，温润明亮且醒目。
pub const TERRACOTTA_BRIGHT: Color = Color::Rgb(242, 142, 102);
pub const SUCCESS: Color = Color::Rgb(140, 172, 137);
pub const DANGER: Color = Color::Rgb(199, 130, 125);
pub const WARN: Color = Color::Rgb(199, 176, 126);

// ── VS Code Dark+ / Modern 经典语法配色 ──
pub const VSCODE_KEYWORD: Color = Color::Rgb(197, 134, 192);   // 紫色 (fn, let, pub, return, if, match)
pub const VSCODE_CONTROL: Color = Color::Rgb(224, 108, 117);   // 控流深粉
pub const VSCODE_STRING: Color = Color::Rgb(206, 145, 120);    // 橙粉色 "string"
pub const VSCODE_COMMENT: Color = Color::Rgb(106, 153, 85);    // 橄榄绿 // comment
pub const VSCODE_TYPE: Color = Color::Rgb(78, 201, 176);       // 青碧色 Option, Result, Vec, String
pub const VSCODE_NUMBER: Color = Color::Rgb(181, 206, 168);    // 淡青绿 123, 0.42
pub const VSCODE_VARIABLE: Color = Color::Rgb(156, 220, 254);  // 天蓝色 self, args, variables
pub const VSCODE_ATTR: Color = Color::Rgb(86, 156, 214);       // 鲜亮蓝 #[], @decorator
