# ADR 0112：TUI 启动页品牌标改按「面积约 1/3」定栅格，并以实心子像素占比设可读性下限

Status: implemented

日期：2026-11-10
关联：[0105](0105-tui-splash-logo-scale-third.md)（本次修订其线性 1/3 口径与 9×10 / 7×8 尺寸，
其余「单一真值源、改生成参数不改渲染层」的结论仍现行）、
[0098](0098-tui-splash-master-truecolor-bitmap-first.md)（覆盖率表 + 母版真色表 + 位图优先，不变）、
[0080](0080-tui-splash-protocol-env-only-detection.md)（零 I/O 协议判定，不变）。
实施说明：[`.agents/notes/implemented/bug-fix/2026-11-10-tui-splash-small-size-legibility.md`](../../.agents/notes/implemented/bug-fix/2026-11-10-tui-splash-small-size-legibility.md)。
真值源仍为 `src-tauri/icons/icon-master-approved.png`。

## 背景

ADR 0105 把品牌标按**线性** 1/3 缩到 Rich 9 列 × 10 子像素（14 行终端里的 5 行品牌画，面积仅原 1/9）。
上线后实机观感是「缩成一团模糊的色块」：折带轮廓、内部负形与暖金高光全部消失。

## 根本原因

模糊不是抗锯齿做多了，而是**栅格低于可读性下限**：面积平均后没有任何一格是「实心」的。

对母版做面积重采样（与 `scripts/generate-splash-logo.py` 同口径），统计每档覆盖率分布：

| 档位 | 栅格 | 最大覆盖率 | 实心子像素（≥0.95） | 结论 |
| --- | --- | --- | --- | --- |
| 现行（0105）Rich | 9×10 | 0.94 | **0** | 全格都是画布色与母版色的半透明混合 |
| 现行（0105）Compact | 7×8 | 0.88 | **0** | 同上 |
| 本次 Rich | 14×16 | 1.00 | 20 / 224 = **8.9%** | 折带主干与高光重新变实 |
| 本次 Compact | 11×12 | 0.96 | 5 / 132 = **3.8%** | 保留内部负形，边缘仍软 |

9 列时母版折带宽（约标宽 7.5%）只有 0.7 个子像素——比一个子像素还窄。覆盖率永远到不了 1，
于是渲染层「按覆盖率向画布色渐隐」的抗锯齿变成把**每一格都稀释成半透明**：颜色互相平均、
没有实心墨，视觉上就是一团模糊。抗锯齿是逐子像素的正确做法，问题在于没有子像素可抗。

同时暴露一处口径错误：用户说的「1/3」在 0105 里按**线性**理解（面积 1/9），
而面积 1/3 对应线性 ≈ 0.58（14–15 列），才是「小一档但仍认得出」的位置。

## 候选方案

- **A（采纳）面积约 1/3 + 实心子像素门禁**：`TIERS` 改 `RICH 14×16` / `COMPACT 11×12`
  （面积 224/780 = 28.7%、132/440 = 30.0%；设备像素 112×128、88×96，宽高比 0.875 / 0.917
  vs 母版 0.895）。新增门禁 `splash_tiers_retain_solid_ink_after_scaling`：两档实心子像素
  （覆盖率 ≥ 242/255）占比必须 ≥3%，让「再往下缩就红」这条可读性下限成为机器可查的承诺。
- **B（落选）回退到 ADR 0098 的 26×30 / 20×22**：可读性最好（实心占 21%），但把 0105 解决的
  「占屏近半」重新引入，与用户诉求相反；且 1/3 已由用户明确要求。
- **C（落选）保持 9×10，改渲染层（更强 AA / SDF 膨胀 / 提亮）**：栅格内没有实心墨，
  SDF 只能把糊块「膨胀」，折带负形与双色走向仍然丢失；等于用滤波掩盖分辨率不足。
- **D（落选）保留面积 1/3 但重画一枚「小尺寸专用母版」**：能更清晰，但真值源会从官方母版
  变成手绘小图，违背 0098「母版即真值」；留作将来的独立提案。
- **E（落选）删除品牌画**：启动页身份缺失，属过度收缩（同 0098 E / 0105 E）。

## 决策

1. `scripts/generate-splash-logo.py` 的 `TIERS` 改为 `[("RICH", 14, 16), ("COMPACT", 11, 12)]`，
   重跑脚本生成三份产物：`splash_coverage.rs`（3205 → 6445 字节）、
   `assets/splash-logo.png`（288×320 → 448×512，42359 → 77802 字节）。
   `SPLASH_IMAGE_COLS/ROWS` 仍由 Rich 档常量派生，位图矩形自动变 14×8 字符。
2. 渲染层零改动：`splash.rs` 只更新尺寸注释；AA 合成、真彩门禁、协议判定、`use_image` 判据全部不动。
3. 高度阶梯阈值保持 28 / 24 行：品牌画 5 → 8 行（Compact 4 → 6 行），
   Rich 构图 17 → 20 行、Compact 16 → 18 行，仍在阈值内。
4. 门禁：新增 `splash_tiers_retain_solid_ink_after_scaling`（两档 ≥3% 实心）；
   尺寸/位图断言更新为 14×16 / 11×12 / 448×512；其余占比类判据按新栅格复核（全部仍通过）。

## 后果

- 观感：折带「A/V」轮廓、内部负形与暖金高光在 14 列下重新可辨，边缘仍保留 AA 渐变。
- 代价：品牌画从 5 行回到 8 行（仍小于 0105 之前的 15 行），内嵌体积 +4.3KB 覆盖率表 / +35KB 位图。
- 可读性下限成为机器承诺：以后任何「再缩小一点」的调整，都会在
  `splash_tiers_retain_solid_ink_after_scaling` 上直接暴露为实心占比不足。
- 撤销点 / 调档方式：只改 `TIERS` 一行并重跑脚本，再同步三条尺寸断言；
  `--check` 会立刻发现「只改产物没改脚本」或反之的漂移。

## 验收凭证

实机捕获（同一 tmux 规格 120×36、`COLORTERM=truecolor`、`PONY_TUI_SPLASH_IMAGE_PROTOCOL=off`
强制字符路径，`tools/tui-audit/render_ansi.py` 按 8×16 单元格重绘）：
[`docs/design/tui-splash-legibility-receipt.png`](../design/tui-splash-legibility-receipt.png)
上格为修复前 9×10（糊块），下格为修复后 14×16（折带可辨）。

| 门 | 结果 |
| --- | --- |
| `cargo test --bin pony splash` | 16 passed / 0 failed（含新增实心占比门） |
| `python3 scripts/generate-splash-logo.py --check` | `splash-logo: 一致`（退出 0） |
| `npm run decisions:check` | 111 篇 ADR 状态/目录/索引一致（退出 0） |
| 实机 Rich 字符路径 / halfblocks 位图路径 | 两条路径均完整出图，位图路径更柔和、字符路径边缘带 AA 渐变 |
