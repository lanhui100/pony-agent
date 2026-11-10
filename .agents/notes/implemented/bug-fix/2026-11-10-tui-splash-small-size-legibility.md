# Agent Note: TUI 启动页品牌标小尺寸糊化修复

Status: implemented

配套决策：[ADR 0112](../../../docs/decisions/0112-tui-splash-logo-legibility-floor.md)；
局部修订 [ADR 0105](../../../docs/decisions/0105-tui-splash-logo-scale-third.md)，承接
[ADR 0098](../../../docs/decisions/0098-tui-splash-master-truecolor-bitmap-first.md) 的单一母版与双路径同源约束。

## Problem

ADR 0105 把字符栅格缩到 Rich 9×10 / Compact 7×8 后，细折带被面积平均成半透明格，
内部负形消失，真实终端中整幅图案糊成一片。两档覆盖率 ≥242/255 的子像素均为 0，
有覆盖区域平均覆盖率仅 0.494 / 0.445；既有 15 项测试未覆盖这一结构保真缺陷。

## Decision

1. `scripts/generate-splash-logo.py` 的两档改为 Rich 14×16 / Compact 11×12，
   字符矩形 14×8 / 11×6；面积约仍为 ADR 0098 原图的 28.7% / 30.0%，不回到大图标。
2. 由同一生成脚本重生成覆盖率表、母版真色表和 448×512 PNG；两条渲染路径继续同源，
   不改抗锯齿、配色、协议检测、环境变量与 28 / 24 高度阈值。
3. 新增两档近乎实心子像素 ≥3% 的回归门；实测 Rich 20/224、Compact 5/132。

## Alternatives considered

- **A. 采用约 1/2 线性尺寸**（采纳）：恢复实心结构和负形，同时把面积控制在原图约 1/3。
- **B. 尺寸不变，只增强覆盖率或换滤镜**：无法恢复已被平均掉的间隙，非真彩路径还会更破碎。
- **C. 恢复 26×30 / 20×22**：重新引入 ADR 0105 的占屏问题。
- **D. 只放大位图路径**：字符画回退仍糊，且两条路径尺寸分裂。

## Consequences

- 覆盖率/真色表 3205 → 6445 字节，PNG 42359 → 77802 字节；内嵌体量仍可忽略。
- `cargo test --bin pony splash` 16 项全绿，生成脚本 `--check` 幂等，`cargo build --bin pony` 退出 0。
- tmux 实机确认 Rich 字符画、Rich halfblocks、Compact 字符画均完整出图；更矮终端沿用既有隐藏规则。
- 撤销必须同时回改 `TIERS`、生成物和测试，不能只替换 PNG。
