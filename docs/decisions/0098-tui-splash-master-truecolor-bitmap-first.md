# ADR 0098：启动页品牌标改用「母版真色派生表 + 位图优先（含 halfblocks）」

Status: implemented

> 档位尺寸（Rich 26×30 / Compact 20×22、位图 832×960）曾由 [0105](0105-tui-splash-logo-scale-third.md)
> 缩为 9×10 / 7×8；因小栅格丢失实心细节，现由 [0112](0112-tui-splash-logo-legibility-floor.md)
> 修订为 Rich 14×16 / Compact 11×12、位图 448×512，高度阶梯阈值 28 / 24 不变；
> 本文件中「覆盖率表 + 母版真色表」「位图优先（含 halfblocks）」「零 stdin 判定承接 0080」等部分仍现行。

日期：2026-11-08
真值源：`src-tauri/icons/icon-master-approved.png`（1024²，同 ADR 0085）；
取代 [0085](superseded/0085-tui-splash-logo-fidelity-and-antialiasing.md)；
方案细节与对比图：[docs/design/tui-splash-logo.md](../../design/tui-splash-logo.md)、
[docs/design/tui-splash-options.png](../../design/tui-splash-options.png)

## 背景

ADR 0085 把轮廓与抗锯齿修对了（IoU 0.463 → 1.00，软边 0% → 98.9%），但配色仍是自造的
OKLab 渐变（C2 克制暖色：暖金 → 赤陶 → 紫），与官方母版的配色是**两套不同的色彩叙事**。
实测母版标身 149060 像素：均值 `[160.2, 61.0, 97.0]`、平均彩度 123.3，主色阶为
`[96,0,96]`(7.5%)、`[128,0,96]`(6.5%)、`[224,96,64]`(6.2%)、`[192,32,64]`(6.0%)、
`[160,0,64]`(5.8%) —— 是「紫红丝带 + 暖金高光」，而 C2 渐变里暖金占了整整一端。

同时用户对四版对比图的裁决是：**要第 4 张的观感、第 2 张的大小**。

- 第 4 张（`tui-splash-options.png` 第 4 格）：母版 RGB 直接粘贴（LANCZOS）、满宽构图，
  标身 bbox 624×704、宽高比 0.886、横向填充率 1.000 —— 观感最好，但字号下占 33 列。
- 第 2 张：26 列宽（313×362、宽高比 0.865）—— 尺寸正好，但当年是纯覆盖率二值 + 自造渐变，
  清晰度不足。

结论：缺的不是分辨率，而是**配色真值源**。把母版 RGB 变成生成物真值表，两条渲染路径
（字符画 / 图形协议位图）都从同一张表取色，就能在 26 列的小footprint里拿到第 4 张的观感。

## 候选方案

- **A. 只改位图路径（kitty/sixel/iTerm2），字符画保留 C2 渐变**：落选——两条路径配色会分裂，
  而 tmux / 普通终端走的正是字符画路径，等于只修了少数终端。
- **B. 母版真色派生表 + 位图优先（含 halfblocks），字符画同步改母版真色/满宽**（采纳）。
- **C. 把 C2 渐变色标换成从母版采样的 7 个色标（保留 1 维渐变）**：落选——母版是 2 维彩带
  （沿折带走向变色，不只是对角渐变），1 维色标必然抹平转折处的高光与暗部；
  且仍是「手挑色标」，与「母版即真值」的单一真值源原则冲突。
- **D. 直接放大到第 4 张的 33 列满宽**：落选——用户明确要第 2 张的大小；33 列在 80 列终端会
  挤掉信息列，且矮终端直接掉档。
- **E. 用 sixel/kitty 位图替代字符画，非图形终端保持现状**：落选——见 A；且 ADR 0080 的
  零 I/O 环境判定在 tmux/未知 TERM 下 fail-closed 返回 `None`，等于最需要改善的终端仍走旧路径。

## 决策

1. **真值源扩为「覆盖率表 + 母版真色表」**。`scripts/generate-splash-logo.py` 除
   `RICH_COVERAGE` / `COMPACT_COVERAGE` 外，新增 `RICH_RGB` / `COMPACT_RGB`
   （每子像素 3 字节，遮罩内加权平均，行优先同序）。覆盖率 0 的子像素真色归零，
   使「表错位」可被门禁识别。生成物仍带「禁止手改」头注释，`--check` 幂等不变。
2. **档位尺寸统一为 Rich 26×30 / Compact 20×22**。Rich 从 27 列收到 26 列（对齐第 2 张的
   footprint，宽高比 0.867 vs 母版 0.895，−3.2%），位图随之 832×960（26×15 格 × 8×16 × 4 超采样）。
   矮终端阈值不变（28 / 24 行）。
3. **字符画改取母版真色**：每格仍写半块 `▀`，`fg`/`bg` 分别取上下子像素的
   `mix(CANVAS, master_rgb, coverage)`。真彩门禁（`COLORTERM` 含 `truecolor`/`24bit`）失效时
   退化为「≥50% 覆盖率上满色」的硬阈值，不劣于旧版。删除 `splash_gradient_rgb`、
   `srgb_to_oklab`、`splash_diagonal_t` 与 C2 色标。
4. **位图路径改为「协议优先」，不再排除 halfblocks**。`init_splash_image` 不再按
   `ProtocolType` 提前返回；`PONY_TUI_SPLASH_IMAGE_PROTOCOL` 的 `halfblocks` 由
   「关闭位图」改为**强制 halfblocks 位图**，只有 `off` 关闭位图路径。
   理由：`ratatui-image` 的 halfblocks 是纯 Rust 的 24bit 前后景半块渲染，无需任何终端能力查询
   即可工作，在 26×15 格下正好给出第 4 张的观感；这也让「终端协议无法判定」的环境
   （tmux / 未知 TERM / 本仓 CI）仍能走位图路径。
5. **构图仍由字符行数决定，位图只铺 Rich 档**。位图矩形固定 26×15，与 Compact 的 20×11
   占位不重合，故 `splash_banner` 的 `use_image` 判据保持 `tier == Rich`；Compact 走字符画。
6. **门禁**：`cargo test --bin pony splash` 15 项，覆盖生成物尺寸（含真色表长度）、
   真色表与覆盖率表同序对齐、母版彩度/色相分布未被抹平、真彩门禁、软边合成、
   半块位图不被排除（`splash_protocol_override_keeps_halfblocks_bitmap`、
   `splash_image_init_has_no_protocol_early_return`）、生成脚本 `--check` 幂等。

## 后果

- 两条路径共用一张母版真色表，配色不再分裂；字符画与位图观感一致。
- 位图路径覆盖 halfblocks，普通终端可经 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=halfblocks` 显式启用，
  也是验收凭证的抓手（本机无终端模拟器，协议判定不可用）。
- 26 列宽高比 0.867 与母版 0.895 差 3.2%，属可接受的字形失真（第 2 张 0.865 更远）。
- 生成物体积：`splash_coverage.rs` 19074 字节、`splash-logo.png` 176332 字节（内嵌进二进制）。
- 撤销点：删除 `*_RGB` 表 + 恢复 `splash_gradient_rgb` 即回到 0085 行为，但需同时回滚
  `init_splash_image` 的协议早退与 `halfblocks` 语义。
