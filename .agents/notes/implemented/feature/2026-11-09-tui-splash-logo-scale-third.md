# Agent Note: TUI 启动页品牌标缩为原尺寸 1/3

Status: implemented

配套决策：[ADR 0105](../../../docs/decisions/0105-tui-splash-logo-scale-third.md)
（局部修订 [ADR 0098](../../../docs/decisions/0098-tui-splash-master-truecolor-bitmap-first.md) 的档位尺寸）；
方案与历史实测：[docs/design/tui-splash-logo.md](../../../docs/design/tui-splash-logo.md)。

## Context

启动页品牌标在 Rich 档占 26 列 × 15 行，空态一屏被品牌画吃掉近半，用户实测边缘锯齿明显，
诉求是「缩放为当前的 1/3」。

尺寸不是一个渲染层参数：`scripts/generate-splash-logo.py` 的 `TIERS` 是覆盖率表、母版真色表、
图形协议位图三份产物的共同父节点（ADR 0098 的单一真值源）。只改渲染层会让图形终端与字符画回退
拿到的尺寸分裂，因此必须改生成参数并重跑脚本。

## Decision

1. `TIERS` 由 `[("RICH", 26, 30), ("COMPACT", 20, 22)]` 改为 `[("RICH", 9, 10), ("COMPACT", 7, 8)]`，
   线性 1/3；仍取母版宽高比（0.895）附近的整数栅格：Rich 9 列 × 5 行 = 72×80 设备像素（0.900），
   Compact 7 列 × 4 行 = 56×64（0.875）。
2. 重生成 `splash_coverage.rs`（19074 → 3205 字节）与 `assets/splash-logo.png`（832×960 → 288×320，
   176332 → 42359 字节）。`SPLASH_IMAGE_COLS/ROWS` 由 Rich 档常量派生，代码无需手改。
3. 高度阶梯阈值保持 28 / 24 行（不含在本次诉求内）；缩小后 Rich 构图 17 行、Compact 16 行，余量更充足。
4. 门禁从「绝对个数」改为「占比」，避免下次改尺寸再手调常数：
   `splash_master_rgb_keeps_master_colors` 有效子像素占比 ≥ 20%（实测 Rich 26.7%），
   `splash_master_rgb_differs_from_canvas` 与画布色有差占比 ≥ 90%（实测 Rich / Compact 均 100%）；
   尺寸断言更新为 9×10 / 7×8 / 288×320。

## Alternatives considered

- **A. 两档统一线性 1/3 + 重跑生成脚本**（采纳）：三份产物同源，两条渲染路径尺寸一致，`--check` 幂等不变。
- **B. 面积 1/3（线性 ≈ 0.58，约 15 列 × 9 行）**：视觉上仍是半个屏幕宽，没解决「太大」；口径也与
  「锯齿」这一线性维度不匹配。落选。
- **C. 只改渲染层矩形，保留 26×30 位图与覆盖率表**：位图密度对缩小面积已过采样，看着省事；但字符画回退
  仍按覆盖率表画 26 列，图形/非图形终端尺寸分裂，违背 0098 的单一真值源。落选。
- **D. 只改超采样/滤波器压锯齿，不动尺寸**：不回答「太大」，且 32×64 px/格 的密度本就不是锯齿主因
  （放大的是 footprint，不是像素）。落选，保留为「小而不糊」的后续选项。
- **E. 删除品牌画**：启动页身份缺失，属过度收缩（同 0098 的 D 项）。落选。

## 后果

- 空态 footprint 26×15 → 9×5 字符，留白回到定位语与命令入口；内嵌体积降到约 1/6。
- 观感代价：9 列下折带细节被压平，非真彩终端（硬阈值、无 AA）的半格颗粒感相对更明显。
- 撤销点：回改 `TIERS` 并重跑脚本 + 恢复三条门禁数值；`--check` 会立刻发现只改了产物没改脚本的情况。
- 验收：`cargo test --bin pony splash` 15 项全绿（含 `generate_script_check_is_idempotent`），
  `python3 scripts/generate-splash-logo.py --check` 输出 `splash-logo: 一致`。
