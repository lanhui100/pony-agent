# Agent Note: TUI 启动页品牌标改用「母版真色派生表 + 位图优先（含 halfblocks）」

Status: implemented

配套决策：[ADR 0098](../../../docs/decisions/0098-tui-splash-master-truecolor-bitmap-first.md)
（接替 [ADR 0085](../../../docs/decisions/superseded/0085-tui-splash-logo-fidelity-and-antialiasing.md)）；
方案与对比图：[docs/design/tui-splash-logo.md](../../../docs/design/tui-splash-logo.md)。

## Context

ADR 0085 把启动页品牌标从「手绘像素掩码」换成了「官方轮廓覆盖率重采样 + 双色 alpha 合成」，
轮廓与抗锯齿问题解决（IoU 0.463 → 1.00，软边 0% → 0.561），但用户随后给出两轮新裁决：

1. 「第 4 张的观感」——四版对比图里第 4 张是**直接粘贴母版 RGB** 的满宽位图，母版色彩叙事完整；
2. 「第 2 张的大小」——第 2 张是 26 列 × 15 行的紧凑footprint（第 4 张的 33 列满宽对 110 列终端太宽）。

同时实测母版标身色彩分布，证明 C2 渐变是**另一套色彩叙事**：母版 149060 像素均值 `[160.2, 61.0, 97.0]`、
平均彩度 123.3，主色阶为 `[96,0,96]`(7.5%)、`[128,0,96]`(6.5%)、`[224,96,64]`(6.2%)、`[192,32,64]`(6.0%)、
`[160,0,64]`(5.8%)——紫红丝带 + 暖金高光两条走向；C2（暖金 → 赤陶 → 紫）把它压成 1 维，转折处高光与暗部被抹平。

另一处结构性分裂：位图路径（kitty/sixel/iTerm2）粘贴母版 RGB，字符画路径走 C2 渐变；
而 tmux / 普通终端 / VSCode 走的正是字符画，所以「母版色」只在少数终端上出现。

## Decision

1. **真值源扩展为「覆盖率表 + 母版真色表」**：`scripts/generate-splash-logo.py` 在原有 `*_COVERAGE`
   之外输出 `RICH_RGB` / `COMPACT_RGB`（每子像素 3 字节，遮罩内加权平均；覆盖率 0 处归零，
   避免「空白格却带彩度」的错位歧义）。两条渲染路径同源取色，不再有手挑色标。
2. **尺寸按第 2 张**：RICH `26 × 30` 子像素 = 26 列 × 15 行（宽高比 0.8667，与母版 0.895 差 −3.2%），
   COMPACT 保持 `20 × 22` = 11 行（0.9091，+1.6%）。位图资产同步改为 **832×960**（26×15 格 × 8×16 px × 4 超采样）。
3. **字符画删除渐变**：`splash_gradient_rgb` / `srgb_to_oklab` / `splash_diagonal_t` 全删，
   `pony_banner_lines` 直接取 `master_rgb` 做 `splash_composite_color`。
4. **位图优先，不排除 halfblocks**：`init_splash_image()` 删除 `ProtocolType::Halfblocks` 早退——
   `ratatui-image` 的 halfblocks 是纯 Rust 的 24bit 前后景半块渲染，无需终端能力查询即可工作，
   在 26×15 格下正好给出第 4 张的观感。`PONY_TUI_SPLASH_IMAGE_PROTOCOL` 的 `halfblocks` 语义由
   「关闭位图」翻转为「强制半块位图」，映射收敛为纯函数 `splash_protocol_override`（零 I/O、可单测）。
5. **位图仍只铺 RICH 档**：位图矩形固定 26×15，与 COMPACT 的 20×11 占位不重合，Compact 走字符画。
6. **门禁 15 项**：`cargo test --bin pony splash` 全绿，新增「真色表与覆盖率表同序对齐」
   （`cov == 0 ⇒ 彩度 ≤ 4`）、「母版色相分布未被抹平」（平均彩度 ≥ 90、紫红与暖金各 ≥ 15%）、
   「halfblocks 不被协议早退排除」（纯函数 + 源码级守卫）。

## Alternatives considered

- **A. 只改位图路径（把 832×960 换进去）**：默认路径仍是字符画，tmux / SSH 里用户看到的还是 C2 渐变，
  不满足「第 4 张的观感」。落选。
- **B. 采纳：位图优先（含 halfblocks）+ 字符画同步改母版真色/满宽**：两条路径同源，任何终端都拿到母版色彩；
  halfblocks 让「无 kitty/sixel 的终端」也能吃到位图观感。采纳。
- **C. 从母版采 7 个色标，保留 1 维渐变**：仍是手挑色标，与「母版即真值」冲突，且 7 档在 30 行内色相跳变明显。落选。
- **D. 33 列满宽（第 4 张原始尺寸）**：110 列终端尚可，80 列终端会挤掉右侧信息列，且高度不变时宽高比被拉伸。落选。
- **E. 图形终端专用（保持 halfblocks 关闭）**：等于承认 tmux 用户是二等公民，而 halfblocks 恰恰是纯 Rust 可用的。落选。

## 后果

- 启动页在**所有**终端上与桌面 App 图标同形同色；母版的紫红丝带 + 暖金高光完整保留（平均彩度 124.7 vs 母版 123.3）。
- 生成物体积：覆盖率 780 + 440 字节，真色表 2340 + 1320 字节，位图 176332 字节（编译期嵌入，可忽略）。
- 26 列 vs 母版 0.895 有 3.2% 的字形失真，属「第 2 张大小」的既定代价（第 2 张本身是 0.865）。
- 撤销点：若母版图标更新，重跑 `scripts/generate-splash-logo.py` 即可，`--check` 与三项真色门禁同时把关。
- halfblocks 的验收抓手是 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=halfblocks`：本机无终端模拟器、
  `COLORTERM` 为空、tmux 不导出 `TERM`，故实机凭证必须显式注入环境变量。
