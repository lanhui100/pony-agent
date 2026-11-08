# Agent Note: TUI 启动页品牌标改为「官方轮廓覆盖率重采样 + 双色 alpha 合成」

Status: implemented

配套决策：[ADR 0085](../../../docs/decisions/superseded/0085-tui-splash-logo-fidelity-and-antialiasing.md)（已由 [ADR 0098](../../../docs/decisions/0098-tui-splash-master-truecolor-bitmap-first.md) 接替）；
方案与对比图：[docs/design/tui-splash-logo.md](../../../docs/design/tui-splash-logo.md)。

## Context

启动页左侧彩色折角标被反复反馈「丑、不平滑」，实测是四条叠加缺陷而非审美分歧：

1. `SPLASH_LOGO_SHAPE` 是 28×22 手绘像素掩码，与官方折带轮廓 IoU 仅 0.463，宽高比 1.273 vs 0.896（横向拉伸 1.42×）；
2. 逐子像素二值填充 ⇒ 零抗锯齿，软边占比 0%；
3. 28×22 子像素下折带宽只剩约 1.7 子像素，细节必然糊；
4. 8 个满饱和色标 + 旧掩码遗留的 `2/19`、`15/19` 魔法数归一化，与 `mod theme`「单色为骨、拒绝高饱和撞色」冲突。

另外 Rust 掩码（28×22）、生成脚本母版（20×16）、`assets/splash-logo.png`（像素画 trace）三份实现互不同源，改一处必漂移。

约束：终端每格只有 fg/bg 两支画笔，画不了真 alpha；但全屏铺不透明 `theme::CANVAS`，所以「向画布色合成」是精确可算的。

## Decision

1. **唯一真值源**：轮廓全部来自 `src-tauri/icons/icon-master-approved.png` 的面积重采样。
   `scripts/generate-splash-logo.py` 产出覆盖率表 `tui/splash_coverage.rs`（带生成头，禁手改）与同源位图，
   支持 `--check` 幂等校验（实测 `splash-logo: 一致`）。手绘掩码与像素画 trace 路径删除。
2. **实现外移**：`tui/splash.rs`（474 行：渐变 / 合成 / 协议判定 / 阶梯 / 品牌画），`tui.rs` 只留 `mod splash;`
   与调用点（净 −414 行），覆盖率数据单列 `tui/splash_coverage.rs`（65 行）。
3. **抗锯齿**：每格固定写 `▀`，`fg = mix(CANVAS, grad(top), α_top)`、`bg = mix(CANVAS, grad(bottom), α_bot)`，
   α 取自覆盖率表。真彩门禁 `COLORTERM` 含 `truecolor`/`24bit` 才启用（纯函数、零 I/O）；
   否则退化为硬阈值，观感不劣于改动前（软边占比 RICH 档实测 0.5607）。
4. **保真比例**：不再横向拉伸，宽高比锚定 master 标记 bbox 的 0.895，按可用高度分档：RICH 27×30 = 15 行
   （0.9000，误差 +0.56%）、COMPACT 20×22 = 11 行（0.9091，误差 +1.57%）、更矮隐藏；
   `splash_banner` 阈值 28 / 24，宽度不足时 RICH 先降 COMPACT。
5. **渐变**：保留 OKLab 插值，`t` 改为标记 bbox 的 45° 轴投影，删除魔法数归一化；
   色标拍板 C2 三档 `#FFC478 → #E96E54 → #965CDC`。
6. **图形协议路径**：保留，位图由同一 master 生成（保比例 contain + CANVAS 底），仅 RICH 档且宽度足够时铺。
7. **门禁**：`cargo test --bin pony splash` 13 项（覆盖率表尺寸/来源、位图可解码、投影跨越 bbox、
   C2 端点与彩度、单通道步进、真彩 env 矩阵、非真彩无 AA、软边存在、行数/列宽不越界、脚本 `--check` 幂等）。

## Alternatives considered

- **A. 提高手绘掩码分辨率（28×22 → 40×34）**：真值源仍不是官方轮廓，IoU 不会变；行高从 11 涨到 17，布局更挤。
- **B. 只修图形协议位图路径**：默认路径是字符画，tmux / VSCode / Windows Terminal 走的正是最丑的那条路径。
- **C. SVG 矢量 + 运行时解析**：引入解析依赖与曲线求交复杂度；1024² 母版的面积重采样精度已足够，
  且生成物可版本化、可 `--check` 防漂移。
- **D. 删除品牌画，只留一句话 + 命令入口**：启动页身份缺失，属过度收缩。
- **E. 采纳**：官方轮廓覆盖率重采样 + 每格双色 alpha 合成。
- **C1 品牌连续渐变（6 档满饱和）落选**：与 `theme::*` 的克制调性冲突，档间色相跳变更明显。

## 后果

- 启动页品牌标与桌面 App 图标同形同色，边缘向画布色渐隐；小终端有明确降级阶梯。
- 消除「Rust 掩码 / Python 母版 / PNG 位图」三份不同源实现，改品牌只需一份 master + 跑脚本。
- 增量成本：启动期多读一个环境变量；覆盖率表 810 + 440 字节数据，可忽略。
- 风险：非真彩终端 AA 关闭后观感与改动前持平；15 行品牌画对极矮终端依赖降级阶梯，
  已由 `splash_banner_grid_matches_tier` 锁住「行数/列宽不越界、不压底部呼吸行」。
