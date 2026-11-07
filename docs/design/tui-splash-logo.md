# TUI 启动页品牌标优化方案

> 状态：已实施（ADR 0085 implemented）。配套决策：[ADR 0085](../decisions/0085-tui-splash-logo-fidelity-and-antialiasing.md)。
> 对比图：[tui-splash-options.png](tui-splash-options.png)
> 落地验收图：[tui-splash-receipt.png](tui-splash-receipt.png)（官方 master / 实机 tmux 字符画 / 同源位图 三联）

## 1. 现象

启动页（`pony` 空态）左侧的彩色折角标「丑、不平滑」：斜边是明显的 1 格台阶，形状读不出品牌，
两段图形之间还有断口感。不是审美偏好问题，是四条可量化的缺陷叠加。

## 2. 量化诊断

以官方 master 图标 `src-tauri/icons/icon-master-approved.png`（1024²，与 `512x512.png` 同源）为真值源：

| 指标 | 现状 | 官方标 / 目标 |
| --- | --- | --- |
| 宽高比 | 1.273（28 列 × 22 子像素） | 0.896（**横向拉伸 1.42×**） |
| 与官方轮廓 IoU | **0.463** | ≥ 0.85 |
| 软边像素占比（0.05<α<0.95） | **0%**（1-bit 判定） | ≥ 40% |

根因：

1. **真值源不是矢量**。`SPLASH_LOGO_SHAPE` 是 28×22 手工像素掩码（注释自称由 20×16 母版上采样），
   与官方折带轮廓只重合 46%：内三角负形、圆角折弯、水平连接段全部丢失，还整体横向拉宽 42%。
2. **没有抗锯齿**。逐子像素只做 `#`/`.` 二值填充，`▀▄` 只表达「有没有」，无法表达「有多少」；
   官方轮廓上 58% 的边缘子像素落在半覆盖区，被硬阈值抹成 1 格宽的台阶。
3. **分辨率不足**。28 列 × 11 行 = 28×22 子像素，官方折带宽约为标宽的 7.5%，落到当前栅格只剩约 1.7 个子像素，
   细节必然糊成一团。
4. **渐变与设计令牌冲突**。8 个满饱和色标横跨整幅画布对角取样，单臂内部色差过大，接缝处色相跳变；
   `t` 归一化还留着旧掩码的魔法数 `2/19`、`15/19`；与 `mod theme` 的「单色为骨、拒绝高饱和撞色」相反。
5. **双实现漂移**。Rust 掩码（28×22）、`scripts/generate-splash-logo.py`（20×16 母版）、
   `assets/splash-logo.png`（616×484，由像素画 trace + Chaikin 拟合）三者不同源，改一处必漂移；
   位图路径的折弯现在是这条 Chaikin 拟合出来的「波浪边」。

## 3. 方案

### A. 字符路径抗锯齿（收益最大，必做）

终端每格只有 fg/bg 两支画笔，但**画布不透明且全屏铺 `theme::CANVAS`**，因此可以精确做 alpha 合成：

- 由官方 master 图标面积重采样（box filter）得到 `cols × rows` 的 u8 覆盖率表，编译期嵌入；
- 每格固定写 `▀`：`fg = mix(CANVAS, grad(top), α_top)`，`bg = mix(CANVAS, grad(bottom), α_bot)`；
- 边缘不再是 0/1 台阶，而是向画布色渐隐——这就是终端里能做到的真实抗锯齿。
- 门禁：仅 `COLORTERM` 含 `truecolor`/`24bit` 时启用（与 `--doctor` 的颜色自检同口径，
  实现为纯函数、零 I/O，沿用 ADR 0080「启动期不碰 stdio」的约束）；否则退回硬阈值，老化终端不退化。

### B. 形状与比例回归品牌标（必做）

- 轮廓一律来自 master 图标的重采样，不再手绘掩码；宽高比 0.896，禁止横向拉伸。
- 尺寸阶梯（按可用高度自适应；宽高比与 master 标记 bbox 的误差 ≤ 3%）：
  - 富余：27 列 × 30 子像素 = **15 行**（宽高比 0.9000，误差 +0.56%）
  - 紧凑：20 列 × 22 子像素 = **11 行**（宽高比 0.9091，误差 +1.57%）
  - 更矮：按现有规则隐藏品牌画（保「一句话 + 命令入口」）
- 布局影响：品牌画 11 行 → 15 行，`show_banner` 行阈值 24 → 28（紧凑档仍为 24）；需同步复核 `CHAT_BOTTOM_GAP` 与 `info_top`。

### C. 渐变（已拍板：C2 克制暖色）

- **C2 克制暖色（采纳）**：3 档（暖金 `#FFC478` → 赤陶 `#E96E54` → 紫 `#965CDC`），与 `mod theme` 的
  「单色为骨 + 单一强调色」一致，红紫中段不串色。对比图右二即 C2。
- **C1 品牌连续渐变（落选）**：按官方图标取色（奶白→琥珀→朱红→洋红→紫→靛），6 档满饱和，
  与 `theme::*` 的克制调性冲突，且档间色相跳变更明显。
- 共同口径：`t` 取标记 bbox 的 45° 轴投影（`0.4·x + 0.6·y`），OKLab 插值 + 逐段 smoothstep，
  删除 `2/19`、`15/19` 魔法数归一化。实测 RICH 档 `t ∈ [0.092, 0.908]`，
  端点 `[253,187,116]` / `[156,96,211]`，`min_chroma = 0.115`，单通道步进 ≤ 2/256。

### D. 图形协议路径（可选，建议同源或删除）

- 若保留：`splash-logo.png` 由同一 master 生成（保比例 contain + CANVAS 底色），尺寸按 `cell_px × cols/lines`
  计算，交由 picker 缩放；删除现在这条像素画 trace。
- 若删除：AA 字符画已足够好，可去掉 `ratatui-image` 与 `assets/splash-logo.png` 的一份双实现，
  代价是 kitty/iTerm 上少一点锐度。

## 4. 实施步骤（提交粒度）

1. 重写 `scripts/generate-splash-logo.py`：输入 master 图标，输出覆盖率表（`tui/splash_coverage.rs`，带生成头注释）
   与（若保留）`assets/splash-logo.png`；`--check` 做漂移检测，接入 verify。
2. `tui.rs`：删除 `SPLASH_LOGO_SHAPE` / `SPLASH_T_MIN` / `SPLASH_T_SPAN`，改 `splash_coverage` 模块 + 覆盖率合成；
   `pony_banner_lines` 由 (cols, rows) 派生，`info_top` 重算。
3. 渐变：`splash_gradient_rgb` 保留 OKLab，换色标与 `t` 归一化。
4. 布局：`show_banner` 阈值、尺寸阶梯、宽度不足降档。
5. 门禁：改 3 个现有 splash 测试；新增 (a) 覆盖率表尺寸/来源校验，(b) 软边存在性（部分像素 != CANVAS 且 != 纯色），
   (c) 行宽/行数不越界，(d) 非真彩不产生 AA 色，(e) 生成脚本 `--check` 幂等。
6. 文档：ADR 0085 转 implemented，本文件改为事实描述。

实施落点：`crates/pony-agent-core/src/bin/tui/splash.rs`（合成 / 渐变 / 协议判定 / 阶梯）、
`tui/splash_coverage.rs`（生成物，禁手改）、`scripts/generate-splash-logo.py`（母版 → 覆盖率表 + 位图）、
`tui/tests/splash_gradient_tests.rs`（门禁）。

## 5. 验收（物理凭证）

实测（`cargo test --bin pony splash`，13 项全绿；`npm run size:check` 0 新增违规）：

| 指标 | 验收线 | 实测 |
| --- | --- | --- |
| 覆盖率表尺寸 | RICH 27×30 / COMPACT 20×22 | 810 / 440 字节，与声明维度一致 |
| 宽高比误差 | ≤ 3% | RICH 0.9000（+0.56%）、COMPACT 0.9091（+1.57%） |
| 软边占比 | ≥ 40% | RICH 0.5607 |
| 渐变端点 | C2 `#FFC478` / `#965CDC` | `[255,196,120]` / `[150,92,220]`，`min_chroma 0.115` |
| 单通道步进 | ≤ 6 / 256 | 2 / 256 |
| 品牌画行数 | ≤ 15 行 | RICH 15 行、COMPACT 11 行，不压底部呼吸行 |
| 非真彩 | 不产生 AA 色 | `COLORTERM` 缺省 → 硬阈值路径，13 例 env 门禁测试全绿 |
| 生成物漂移 | `--check` 幂等 | `splash-logo: 一致` |
| 实机 IoU | ≥ 0.85 | **1.00**（tmux 捕获 27×30 反解 alpha vs master） |
| 实机软边 | ≥ 40% | **0.989**（覆盖率非 0/1 的边界子像素中，捕获值同样为中间值） |
| 合成残差 | ≤ 1/256 | 单通道 ≤ 1（反解 alpha 与 master 覆盖率平均绝对差 0.0004） |

捕获方式：`tmux new-session -x 110 -y 40` 内跑 `COLORTERM=truecolor target/release/pony`，
`capture-pane -e` 取带 SGR 的 40 行，按 8×16 单元格重绘（`docs/design/tui-splash-receipt.png` 左中右三联）。

## 6. 风险

- **非真彩终端**：AA 关闭后回到硬边，观感与现在持平，不劣化（这也是 A 必须带门禁的原因）。
- **高度预算**：15 行品牌画在 30 行终端仍可容纳；更矮时降档或隐藏，需保证不压输入带。
- **审美方向**：已拍板 C2；色标若再改，须同时跑生成脚本 + 三项渐变门禁，避免只改 Rust 造成漂移。
