# Agent Note: TUI 启动页品牌标改为「官方轮廓覆盖率重采样 + 双色 alpha 合成」

Status: archived (原状态: superseded)

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0085-tui-splash-logo-fidelity-and-antialiasing.md` 之归档快照，仅供溯源。

日期：2026-11-07
真值源：`src-tauri/icons/icon-master-approved.png`（1024²，与 `icons/512x512.png`、`icons/icon.png` 同源）；
方案细节与对比图：[docs/design/tui-splash-logo.md](../../design/tui-splash-logo.md)、
[docs/design/tui-splash-options.png](../../design/tui-splash-options.png)

## 背景

启动页左侧彩色折角标被反复反馈「丑、不平滑」。实测不是审美分歧，而是四条叠加缺陷：

1. **形状不是品牌标**。`SPLASH_LOGO_SHAPE` 是 28×22 手绘像素掩码（注释称由 20×16 母版上采样），
   与官方折带轮廓的 IoU 仅 **0.463**，宽高比 **1.273 vs 0.896（横向拉伸 1.42×）**；
   内三角负形、圆角折弯、水平连接段全部丢失。
2. **零抗锯齿**。逐子像素二值填充，软边占比 **0%**；官方轮廓 58% 的边缘子像素处于半覆盖区，
   被硬阈值抹成 1 格宽台阶——这正是「不平滑」的直接来源。
3. **分辨率预算不足**。28×22 子像素下，官方折带宽（约标宽 7.5%）只剩约 1.7 个子像素，细节必然糊。
4. **渐变与设计令牌反着来**。8 个满饱和色标跨整幅画布对角取样 + 旧掩码遗留的 `2/19`、`15/19` 魔法数
   归一化；`mod theme` 的设计前提是「单色为骨、拒绝高饱和撞色」，两者直接冲突。
   另外 Rust 掩码（28×22）、生成脚本母版（20×16）、`assets/splash-logo.png`（616×484，像素画 trace +
   Chaikin 拟合）三份实现互不同源，改一处必漂移。

约束：终端每格只有 fg/bg 两支画笔，无法逐像素画 alpha；但整个 UI 全屏铺不透明 `theme::CANVAS`，
因此「向画布色合成」是精确可算的，不需要真正的透明度。

## 候选方案

- **A. 提高手绘掩码分辨率（28×22 → 40×34）**：落选——真值源仍不是官方轮廓，IoU 不会变；
  行高还从 11 涨到 17，布局更挤。
- **B. 只修图形协议位图路径（kitty/iTerm/sixel）**：落选——默认路径是字符画，
  tmux/VSCode/Windows Terminal 等占绝大多数的终端走的正是最丑的那条路径。
- **C. 用 SVG 矢量 + 运行时解析**：落选——引入解析依赖与曲线求交复杂度；master 图标 1024² 的面积
  重采样精度已经足够，且生成物可版本化、可 `--check` 防漂移。
- **D. 删除品牌画，只留一句话 + 命令入口**：落选——启动页身份缺失，属过度收缩。
- **E. 采纳：官方轮廓覆盖率重采样 + 每格双色 alpha 合成**。

## 决策

1. **唯一真值源**：轮廓一律来自 `icon-master-approved.png` 的面积重采样；手绘掩码与
   `assets/splash-logo.png` 的像素画 trace 路径已删除。生成脚本 `scripts/generate-splash-logo.py`
   输出覆盖率表（`tui/splash_coverage.rs`，带生成头注释）与同源位图，并支持 `--check` 幂等校验
   （实测 `splash-logo: 一致`）。
2. **字符路径抗锯齿**：每格写 `▀`，`fg = mix(CANVAS, grad(top), α_top)`、`bg = mix(CANVAS, grad(bottom), α_bot)`；
   α 来自覆盖率表。真彩门禁：`COLORTERM` 含 `truecolor`/`24bit` 才启用（纯函数、零 I/O，沿用 ADR 0080），
   否则退化为硬阈值，保证不劣于现状（实测软边占比 RICH 档 0.5607）。
3. **保真比例**：不再横向拉伸，宽高比锚定 master 标记 bbox 的 0.895；尺寸阶梯按可用高度自适应
   （富余 27×30 = 15 行，比例误差 +0.56% / 紧凑 20×22 = 11 行，误差 +1.57% / 更矮隐藏），
   `show_banner` 阈值随之重算（28 / 24）。宽度不足时 Rich 先降 Compact。
4. **渐变**：保留 OKLab 插值，`t` 改为标记 bbox 的 45° 轴投影（删除 `2/19`、`15/19` 魔法数归一化）；
   色标拍板 **C2 克制三档暖色**（`#FFC478` → `#E96E54` → `#965CDC`，与 `mod theme` 一致），
   落选 C1 品牌连续渐变（6 档满饱和与设计令牌冲突）。
5. **图形协议路径**：保留。位图由同一 master 生成（保比例 contain + CANVAS 底，`emit_png` 不再粘贴
   master RGB，渐变即真值，故位图与字符路径同色同形）；仅在 Rich 档且宽度足够时铺，其余走字符画。
6. **门禁**：`cargo test --bin pony splash` 共 13 项——覆盖率表来源/尺寸、位图可解码且为 864×960、
   对角投影跨越 bbox、C2 端点与 `min_chroma ≥ 0.11`、单通道步进 ≤ 6、真彩门禁 13 例 env 矩阵、
   非真彩无 AA、软边占比 ≥ 15%（实测 56%）、品牌画行数/列宽不越界、`--check` 幂等。
   落地后 `npm run size:check` 0 新增违规。

## 后果

- 启动页品牌标与桌面 App 图标同形同色，边缘向画布色渐隐；小终端有明确的降级阶梯。
- 已消除「Rust 掩码 / Python 母版 / PNG 位图」三份不同源实现，改品牌只需一份 master + 跑脚本。
- 增量成本：启动期多读一个环境变量（无 I/O 风险）；覆盖率表为 810 + 440 字节数据，可忽略。
- 风险：非真彩终端 AA 关闭后观感与改动前持平；15 行品牌画对极矮终端依赖降级阶梯，
  已由 `splash_banner_grid_matches_tier` 锁住「行数/列宽不越界、不压底部呼吸行」。

## 可观测性归属（N-A，结构性理由）

四维全部维持 N-A：本次只改**本地渲染数据与合成算法**（覆盖率表、渐变投影、`▀` 双色合成、尺寸阶梯），
不新增对外接口、网络 IO、后台任务、LLM 调用，也不改动既有 `error_boundary` / `http_client` 收口点。
唯一新增运行时读取是启动期一次 `COLORTERM` 环境变量判断（纯函数、零 I/O），不产生遥测事实，
因此 `docs/observability.yaml` 无需新增登记项；行为门禁落在测试层
（`cargo test --bin pony splash` 13 例 + 生成脚本 `--check` 幂等）。

