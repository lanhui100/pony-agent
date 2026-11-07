# Agent Note: 修复 a2 主题对比度用例在 `tui.rs` 模块化后失效（源码扫描漏掉 `tui/theme.rs`）

Status: implemented

## Context

`99b3adce refactor(tui): tui.rs 原地模块化拆分（Step 1+2）` 把设计令牌从 `src/bin/tui.rs` 的
内联 `mod theme` 机械外移为 `src/bin/tui/theme.rs`，`tui.rs` 只保留 `mod theme;`。

`crates/pony-agent-core/tests/tui_audit_scenarios_regression.rs` 的
`a2_contrast_ratio_matches_wcag_and_theme_colors_are_readable` 用「源码里能 grep 到
`Color::Rgb(r, g, b)`」来自证 WCAG 基线常量与源码同源。它只 `read_to_string(tui.rs)`，
拆分后 4 个基线色（FAINT / GHOST / ACCENT_DIM / CANVAS）全部从被扫描文件中消失，用例报
「A2: 源码中找不到 FAINT 的 RGB(124, 118, 110)」。

这不是色值漂移，而是**校验对象被搬走**：真正的色值仍在（`theme.rs:14` 等），只是扫描范围没跟上。
在干净 HEAD 树（`/tmp/wt-splash` @ `fa76a170`，无任何未提交改动）单跑该用例同样失败
（`A2_EXIT=101`），证明这是 `99b3adce` 引入的既有结构性红，与启动页品牌标改造、
与工作区他人在飞的 `plan_state.rs` 改动均无关。

## Decision

把对账范围从「单个文件」改为「`tui.rs` + `tui/theme.rs`」逐文件读取后拼接再匹配：

- 任一文件读不到 → panic（fail-closed，模块被再次搬家时用例立刻红，而不是静默放行）；
- 匹配语义不变（仍是字面量 `Color::Rgb(r, g, b)`），基线常量仍是唯一真值。

只动测试，不动产品代码与色值。

## Alternatives considered

1. **把色值改回 `tui.rs` 内联** — 否决：与 `99b3adce` 的模块化方向和 `scripts/check-file-size.mjs`
   的代码规模门禁背道而驰，为迁就测试而回退架构。
2. **用 `include_str!("../src/bin/tui/theme.rs")` 编译期常量** — 否决：`theme.rs` 是 `tui.rs`
   的子模块而非独立 crate，测试二进制不编译它；`include_str!` 需要相对测试文件的路径硬编码，
   且失去「读不到就 panic」的运行期 fail-closed 语义。
3. **递归扫描整个 `src/bin/tui/` 目录** — 否决：会让用例对任意新增文件里的同值字面量误判为「已对账」，
   校验强度下降；显式白名单文件更符合「一个事实一个家」。
4. **删除这四条源码对账断言，只保留 Python 侧 WCAG 计算** — 否决：那部分只校验对比度算式，
   丢掉「基线常量 == 实际主题色」的绑定，正是本用例存在的理由。

## Evidence

- 干净 HEAD `fa76a170`：`cargo test -p pony-agent-core --test tui_audit_scenarios_regression a2_`
  → `A2_EXIT=101`（红在 HEAD）。
- 修复后工作区：`cargo test -p pony-agent-core --test tui_audit_scenarios_regression`
  → `3 passed; 0 failed`（a1 / a2 / a3 全绿）。
- `cargo fmt` 未对本文件报差异。
