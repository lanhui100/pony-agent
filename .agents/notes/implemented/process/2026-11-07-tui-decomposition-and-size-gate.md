# Agent Note: tui.rs 模块化拆分（Step 1+2）与代码规模门禁棘轮

Status: implemented

## Context

`crates/pony-agent-core/src/bin/tui.rs` 拆分前 15336 行，同时承担终端恢复、输入/队列判定、
命令解析、`TuiState` 状态机、渲染与 token、轮次编排与事件循环、CLI 入口六类职责；
`tui_event_loop` 2320 行 / 嵌套 16 层，`ui()` 875 行并直写 14 处状态字段。
仓库当时没有任何单文件/单函数规模护栏，单日净增约 2900 行。

硬约束：35+ 个 `crates/pony-agent-core/tests/*.rs` 用 `#[path = "../src/bin/tui.rs"] mod tui;`
把该 bin 当模块编译，任何可见性收缩都会一次性打断全部测试目标。

## Decision

1. 原地模块化（详见 ADR 0089）：根文件保留 `main` 与模块声明，实现下沉 `src/bin/tui/`，
   统一 `#[path = "tui/xxx.rs"]`（与既有 `protocol.rs` 同构），根侧 `pub use self::xxx::*`
   维持 `tui::<item>` 名称空间 ⇒ 测试目标零改动。
2. 已外移 9 个实现模块（theme/errors/text/markdown/input/queue/terminal/selection/projection，
   共 2495 行）与 9 个测试模块（共 2080 行，断言与 HEAD 内联版本逐行一致）：
   根文件 15336 → 11095 行（-4241 行 / -27.7%）。
3. 事件循环 reducer 化与 `ui()` widget 化（Step 3）**未做**，需先扩充或新增 ADR 再动手。
4. 规模门禁走「基线棘轮」（详见 ADR 0090）：`scripts/check-file-size.mjs` 全仓扫描
   文件 ≤3000 行 / 函数 ≤200 行 / 嵌套 ≤8 层；基线 `scripts/file-size-baseline.json`
   记录 70 条存量超限，只判「超出基线」的新增/恶化；`npm run verify` 已接入 `npm run size:check`。

## Alternatives considered

- **不拆，只加注释**：落选。编译反馈成本不变（`cargo check --bin pony` 实测 78.66 s），
  也拦不住增长。
- **拆独立 crate `pony-tui`**：落选（本轮）。收益是编译单元隔离，但要同时搬运 `#[path]` 测试可见性面
  与 `src-tauri` 的间接依赖，属拆分 + 换架构双重变更，回滚成本高。
- **换 TUI 框架 / 异步重写**：落选。与降低单文件耦合正交，会让 40 个测试目标语义漂移，无法逐步验收。
- **全仓硬规模门禁**：落选。70 条存量债会让 `npm run verify` 当天永久红，逼迫重写无关大文件。
- **只扫描 `src/bin/tui*`**：落选。能立刻绿，但其余模块继续无约束增长，扩展范围时仍要面对存量。
- **只做报告、永不失败**：落选。不会失败的检查等于没有检查。

## Consequences

- `tui_event_loop` / `ui()` 仍超阈值，由基线 grandfathered 记账；拆分不得以放宽门禁换绿。
- 基线键含同名函数出现序号（`projection.rs` 两个 `apply` 388/223 行），避免互相覆盖造成假通过。
- 新增超限或文件变长 → `npm run size:check` 退出码 1；豁免围栏（`// ponylint:file-size-exempt`）
  过期或缺理由一律失败且不进基线。
- 已知边界：词法级近似判定；不覆盖 TS/Vue 侧与「单文件关注点数量」。
