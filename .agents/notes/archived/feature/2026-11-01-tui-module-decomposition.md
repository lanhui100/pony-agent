# Agent Note: tui.rs 原地模块化拆分，保留 bin 入口与 `tui::` 可见性

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0089-tui-module-decomposition.md` 之归档快照，仅供溯源。

## 背景

`crates/pony-agent-core/src/bin/tui.rs` 在拆分前是 **15336 行**（HEAD 实测）单文件，同时承担六类关注点：
终端/信号恢复、纯输入与队列判定、命令解析与待处理请求、`TuiState` 状态机、渲染与设计 token、
轮次编排 + 事件循环 + CLI 入口（`pony` 的 `main`、doctor、自更新）。其中：

- `tui_event_loop` 约 2320 行、花括号嵌套 16 层；
- `ui()` 875 行，并以 `&mut TuiState` 直写 14 处状态字段（渲染层反写状态）；
- 单日 12 次提交带来 +2900 行增长，仓库当时没有任何单文件/单函数规模护栏。

硬约束：**35+ 个 `crates/pony-agent-core/tests/*.rs` 用 `#[path = "../src/bin/tui.rs"] mod tui;`
把该 bin 当测试模块编译**，测试直接访问 `tui::<item>`。任何改变可见性面的拆分都会一次性打断全部测试目标。

## 候选方案 / Alternatives considered

- **A. 不拆，只补控制流注释**：落选。注释不降低编辑反馈成本（`cargo check --bin pony` 实测 78.66 s
  且 35 个测试 crate 各自重编这 15k 行），也不阻止继续以每天数千行速度增长。
- **B. 拆成独立 workspace crate `pony-tui`，让 bin 只做 `main` 转发**：落选（本轮）。收益是编译单元隔离，
  但要新增 crate、搬运 `#[path]` 测试的可见性面、重设 `Cargo.toml` feature 与 `src-tauri` 的
  `cargo:check` 间接依赖，属「拆分 + 换架构」双重变更，回滚成本高；等 Step 3 完成、模块边界稳定后
  再评估才有依据。
- **C. 换 TUI 框架或改异步重写**：落选。与本次「降低单文件耦合」目标正交；同时改渲染模型会让
  既有 40 个测试目标语义漂移，无法逐步验收。
- **D. 原地子模块化（采用）**：根文件保留 `main` 与模块声明，实现下沉到 `src/bin/tui/`，
  用 `#[path]` 引用、根侧 `pub use` 保持可见性面 —— 单次改动可编译、可回滚、测试零改写。
- **E. 只外移内联测试**：不作为最终方案（采纳为 Step 1）。收益确定但只减 13% 体积，
  不触及 2320 行事件循环这一主要耦合源。

## 决策

1. 根模块 `src/bin/tui.rs` 继续作为 `[[bin]] name = "pony"` 的入口，只保留：模块声明、
   `pub use self::<mod>::*` 兼容层、`main` 与仍待拆的编排代码。
2. 子模块统一用 `#[path = "tui/xxx.rs"] mod xxx;` 引用（与既有 `tui/protocol.rs` 一致）：
   bin 目标与「bin 当模块被 `#[path]` 引入」两种编译形态都能解析到同一份源文件。
3. 根侧 `pub use self::xxx::*;` 维持 `tui::<item>` 名称空间 ⇒ 既有 `tests/*.rs` 的
   `#[path] mod tui;` **零改动**，可见性收缩即视为破坏性变更。
4. 拆分按三步推进，每步独立验收、独立可回滚：
   1. 内联测试外移到 `tui/tests/*.rs`（`#[cfg(test)] #[path]`），断言逐字不改；
   2. 无跨模块可变状态的纯函数层外移（尺寸/文本/输入/队列/投影/终端/选区/错误/Markdown）；
   3. 事件循环按 `UiEvent` 拆 reducer、`ui()` 拆 widget 并把状态写回上移到事件层 —— **需先扩充或新增 ADR 再动手**。
   本 ADR 只覆盖 Step 1 与 Step 2；Step 3 不因本篇存在而自动获批。工程进度由任务板跟踪，不改本状态。
5. 每步验收口径：`cargo check --bin pony` 零 error/零 warning（生产 `-D warnings`），
   且 `cargo test --test tui_acceptance` 通过；全量 `cargo test --no-run` 无编译错误。

## 影响

- 现状：根文件 15336 → 11095 行；外移 9 个实现模块（theme/errors/text/markdown/input/queue/
  terminal/selection/projection）与 9 个测试模块，断言经逐行比对与 HEAD 内联版本完全一致。
- 仍超规模阈值的事实不隐藏：`tui_event_loop` 2320 行、`ui()` 875 行仍在根文件，
  由 ADR 0090 的规模门禁基线如实记账（grandfathered），拆分不得以放宽门禁换绿。
- 子模块普遍 `use super::*;` 依赖根模块的导入面：移动任何 `use` 都要同时跑 bin 与测试目标。
- 保留 `#![cfg_attr(test, allow(dead_code))]` 这一既有豁免；禁止扩展为 `#![allow(warnings)]`。
- 新增模块文件首行必须注明「原 tui.rs 内联内容 + 外移步骤」，便于追溯来源。

