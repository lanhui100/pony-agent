# TUI 启动页品牌标优化方案

> 状态：已实施（ADR 0098 implemented）。配套决策：[ADR 0098](../decisions/0098-tui-splash-master-truecolor-bitmap-first.md)
> （承接已被接替的 [ADR 0085](../decisions/superseded/0085-tui-splash-logo-fidelity-and-antialiasing.md)）。
> **现行尺寸以 [ADR 0112](../decisions/0112-tui-splash-logo-legibility-floor.md) 为准**
> （Rich 14×16 / Compact 11×12，位图 448×512）；[ADR 0105](../decisions/0105-tui-splash-logo-scale-third.md)
> 的 9×10 / 7×8 因实心细节与内部负形丢失已修订。
> 下文「26×30 / 832×960 / 15 行」的量化实测属最初基线，作为当时验收凭证保留，不再代表现行尺寸。
> 对比图：[tui-splash-options.png](tui-splash-options.png)
> 落地验收图：[tui-splash-receipt.png](tui-splash-receipt.png)（官方 master / 实机 tmux 字符画 / 同源位图 三联）
> 糊化修复前后对照：[tui-splash-legibility-receipt.png](tui-splash-legibility-receipt.png)
> （同一 tmux 会话规格 120×36 实机捕获：上 9×10 糊块 / 下 14×16 可辨折带）

## 1. 现象

启动页（`pony` 空态）左侧的彩色折角标「丑、不平滑」：斜边是明显的 1 格台阶，形状读不出品牌，
两段图形之间还有断口感。ADR 0085 修掉了轮廓与抗锯齿，但配色仍是自造的 OKLab 渐变，
与官方母版是两套色彩叙事；四版对比图裁决为「要第 4 张的观感、第 2 张的大小」。

## 2. 量化诊断

以官方 master 图标 `src-tauri/icons/icon-master-approved.png`（1024²，与 `512x512.png` 同源）为真值源：

| 指标 | ADR 0085 前 | ADR 0085 后 | 现行（ADR 0098） |
| --- | --- | --- | --- |
| 宽高比 | 1.273（28 列 × 22 子像素） | 0.900 | 0.867（Rich 26×30） |
| 与官方轮廓 IoU | **0.463** | 1.00 | 1.00 |
| 软边像素占比（标身内 0.05<α<0.95） | **0%** | 0.561 | 0.536 |
| 平均彩度（覆盖率 ≥ 50% 的子像素） | — | C2 渐变自带 | **124.7**（母版 123.3） |
| 紫红 / 暖金子像素占比（b 通道最强 / r 通道最强且非近中性） | — | 渐变单一走向 | **0.294 / 0.316** |

根因（前三条由 ADR 0085 消除，后两条由 ADR 0098 消除）：

1. ~~**真值源不是矢量**~~。旧 `SPLASH_LOGO_SHAPE` 是 28×22 手工像素掩码，与官方折带轮廓只重合 46%。
2. ~~**没有抗锯齿**~~。逐子像素二值填充，官方轮廓 58% 的边缘子像素被抹成 1 格宽台阶。
3. ~~**分辨率不足**~~。28 列 × 11 行下官方折带宽（约标宽 7.5%）只剩约 1.7 个子像素。
4. **配色是自造的**。C2 渐变（暖金 → 赤陶 → 紫）把母版的「紫红丝带 + 暖金高光」压成 1 维走向；
   母版标身 149060 像素实测均值 `[160.2, 61.0, 97.0]`、平均彩度 123.3，主色阶为
   `[96,0,96]`(7.5%)、`[128,0,96]`(6.5%)、`[224,96,64]`(6.2%)、`[192,32,64]`(6.0%)、`[160,0,64]`(5.8%)。
5. **两条渲染路径配色分裂**。位图路径（kitty/sixel/iTerm2）粘贴母版 RGB，字符画路径走 C2 渐变；
   而 tmux / 普通终端走的正是字符画。

## 3. 方案

### A. 字符路径抗锯齿（已实施）

终端每格只有 fg/bg 两支画笔，但**画布不透明且全屏铺 `theme::CANVAS`**，因此可以精确做 alpha 合成：

- 由官方 master 图标面积重采样（box filter）得到 `cols × rows` 的 u8 覆盖率表，编译期嵌入；
- 每格固定写 `▀`：`fg = mix(CANVAS, master_rgb(top), α_top)`，`bg = mix(CANVAS, master_rgb(bottom), α_bot)`；
- 边缘不再是 0/1 台阶，而是向画布色渐隐——这就是终端里能做到的真实抗锯齿；
- 门禁：仅 `COLORTERM` 含 `truecolor`/`24bit` 时启用（与 `--doctor` 的颜色自检同口径，
  实现为纯函数、零 I/O，沿用 ADR 0080「启动期不碰 stdio」的约束）；否则退回硬阈值（覆盖率 ≥ 50% 上满色），
  老化终端不退化。

### B. 形状、比例与配色回归品牌标（已实施）

- 轮廓与配色**一律来自 master 图标的重采样**，不再手绘掩码、不再自造渐变：
  `scripts/generate-splash-logo.py` 同时产出覆盖率表与母版真色表（每子像素 3 字节）。
- 尺寸阶梯（按可用高度自适应；宽高比与 master 标记 bbox 的误差 ≤ 5%）：
  - 富余：26 列 × 30 子像素 = **15 行**（宽高比 0.8667，误差 −3.2%）
  - 紧凑：20 列 × 22 子像素 = **11 行**（宽高比 0.9091，误差 +1.6%）
  - 更矮：按现有规则隐藏品牌画（保「一句话 + 命令入口」）
- 布局影响：品牌画 11 行 → 15 行，`show_banner` 行阈值 24 → 28（紧凑档仍为 24）。

### C. 配色（已拍板：母版真色，不再有手挑色标）

- **采纳**：`RICH_RGB` / `COMPACT_RGB` 逐子像素保存母版真色（遮罩内加权平均，覆盖率 0 处归零），
  两条渲染路径同源取色。实测平均彩度 124.7 / 125.8，与母版 123.3 一致；紫红 0.294、暖金 0.316，
  母版的「紫红丝带 + 暖金高光」双走向被完整保留。
- **落选 C1「品牌连续渐变」**：按官方图标取 6 个满饱和色标做 1 维插值，档间色相跳变明显。
- **落选 C2「克制暖色」**（ADR 0085 曾采纳）：3 档暖金 → 赤陶 → 紫，与 `theme::*` 调性一致，
  但把母版 2 维彩带压成 1 维走向，转折处的高光与暗部被抹平。
- **落选 C3「从母版采 7 个色标 + 保留 1 维渐变」**：仍是手挑色标，与「母版即真值」冲突。

### D. 图形协议路径（已实施：位图优先，含 halfblocks）

- `assets/splash-logo.png` 由同一 master 生成（832×960 = 26×15 格 × 8×16 像素 × 4 超采样，
  满宽 contain + CANVAS 底色，母版 RGB 直接粘贴），交由 `Picker` 缩放。
- **不再排除 `ProtocolType::Halfblocks`**：`ratatui-image` 的 halfblocks 是纯 Rust 的 24bit 前后景
  半块渲染，无需任何终端能力查询即可工作；在 26×15 格下正好给出第 4 张对比图的观感。
  只有 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=off` 关闭位图路径；`halfblocks` 由「关闭位图」改为**强制半块位图**。
- 位图矩形固定 26×15，与 Compact 的 20×11 占位不重合，故仅在 Rich 档铺位图，Compact 走字符画。

## 4. 实施步骤（提交粒度）

1. 重写 `scripts/generate-splash-logo.py`：输入 master 图标，输出覆盖率表 + 母版真色表
   （`tui/splash_coverage.rs`，带生成头注释）与 `assets/splash-logo.png`；`--check` 做漂移检测。
2. `tui/splash.rs`：删除 `SPLASH_LOGO_SHAPE` / `SPLASH_T_MIN` / `SPLASH_T_SPAN` / `splash_gradient_rgb` /
   `srgb_to_oklab` / `splash_diagonal_t`，改 `splash_coverage` 模块 + 母版真色合成；
   `pony_banner_lines` 由 (cols, rows) 派生，`info_top` 重算。
3. 配色：字符画改取 `RICH_RGB` / `COMPACT_RGB`，删除 C2 色标与 45° 投影归一化。
4. 布局：`show_banner` 阈值、尺寸阶梯、宽度不足降档。
5. 协议：`init_splash_image` 删除 halfblocks 早退；`PONY_TUI_SPLASH_IMAGE_PROTOCOL` 映射改为纯函数
   `splash_protocol_override`（零 I/O、可独立单测）。
6. 门禁：`cargo test --bin pony splash` 15 项，含 (a) 生成物尺寸与来源校验，(b) 真色表与覆盖率表同序对齐，
   (c) 母版彩度/色相分布未被抹平，(d) 非真彩不产生 AA 色，(e) 半块位图不被排除，(f) 生成脚本 `--check` 幂等。
7. 文档：ADR 0098 转 implemented，ADR 0085 转 `superseded`，本文件改为事实描述。

实施落点：`crates/pony-agent-core/src/bin/tui/splash.rs`（合成 / 真色取用 / 协议判定 / 阶梯）、
`tui/splash_coverage.rs`（生成物，禁手改）、`scripts/generate-splash-logo.py`（母版 → 覆盖率表 + 真色表 + 位图）、
`tui/tests/splash_gradient_tests.rs`（门禁）。

## 5. 验收（物理凭证）

实测（`cargo test --bin pony splash`，15 项全绿；`python3 scripts/generate-splash-logo.py --check` 幂等）：

| 指标 | 验收线 | 实测 |
| --- | --- | --- |
| 覆盖率表尺寸 | RICH 26×30 / COMPACT 20×22 | 780 / 440 字节，与声明维度一致 |
| 真色表尺寸 | 3 × 覆盖率表 | RICH 2340 / COMPACT 1320 字节 |
| 真色表来源 | 由 master 派生、禁手改 | 生成物头注释 + 测试串校验 `icon-master-approved.png` / `禁止手改` |
| 宽高比误差 | ≤ 5% | RICH 0.8667（−3.2%）、COMPACT 0.9091（+1.6%） |
| 软边占比（标身内） | ≥ 40% | RICH 0.536、COMPACT 0.581 |
| 平均彩度 | 与母版 123.3 同档 | RICH 124.7、COMPACT 125.8 |
| 紫红 / 暖金双走向 | 各 ≥ 15% | 0.294 / 0.316 |
| 真色表与覆盖率对齐 | 覆盖率 0 处无彩度 | 0 例越界（覆盖率 0 处真色归零） |
| 品牌画行数 | ≤ 15 行 | RICH 15 行、COMPACT 11 行，不压底部呼吸行 |
| 非真彩 | 不产生 AA 色 | `COLORTERM` 缺省 → 硬阈值路径，13 例 env 门禁测试全绿 |
| 半块位图 | 不被协议早退排除 | `splash_protocol_override_keeps_halfblocks_bitmap` + `splash_image_init_has_no_protocol_early_return` |
| 生成物漂移 | `--check` 幂等 | `splash-logo: 一致` |
| 实机 IoU | ≥ 0.85 | **1.00**（tmux 捕获反解 alpha vs master 覆盖率，ADR 0098 凭证） |
| 实机软边 | ≥ 40% | **0.994**（标身边界子像素中捕获值同为中间值） |
| 字符画合成残差 | ≤ 1/256 | **0/255**：渲染值 vs 生成表合成值逐通道整数残差全为 0 |
| 位图采样残差 | ≤ 2/255 | **≤ 1/255**（`PONY_TUI_SPLASH_IMAGE_PROTOCOL=halfblocks` 捕获 vs `splash-logo.png` 双线性采样，均值 0.02） |

捕获方式：`tmux new-session -x 110 -y 40` 内跑 `COLORTERM=truecolor TERM=xterm-256color target/release/pony`，
`capture-pane -e` 取带 SGR 的行，按 8×16 单元格重绘（`docs/design/tui-splash-receipt.png`）。
本机无终端模拟器、`COLORTERM` 为空、tmux 会话未导出 `TERM`，故凭证必须显式注入这两个变量；
位图路径同理用 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=halfblocks` 显式启用。

## 6. 风险

- **非真彩终端**：AA 关闭后回到硬边，观感与 ADR 0085 前持平，不劣化（这也是 A 必须带门禁的原因）。
- **高度预算**：15 行品牌画在 30 行终端仍可容纳；更矮时降档或隐藏，需保证不压输入带。
- **配色方向**：已拍板「母版即真值」，不再有手挑色标；若母版图标更新，重跑生成脚本即可，
  三项真色门禁会同时把关，不会出现「只改 Rust 造成漂移」。
- **宽高比 0.867 与母版 0.895 差 3.2%**：属字形失真预算内（第 2 张对比图为 0.865）。
