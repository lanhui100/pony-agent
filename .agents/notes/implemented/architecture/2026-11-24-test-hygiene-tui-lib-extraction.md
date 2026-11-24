# Agent Note: test-hygiene 3b-3 —— 抽取 `pony_agent_core::tui` 库模块并清零 `bin-as-module` 棘轮

Status: implemented

## Problem

在 [test-hygiene 3b-1](2026-11-24-test-hygiene-and-cli-lib-extraction.md) 将 `ponycli` 共享核心抽入 `pony_agent_core::cli`、并在 [test-hygiene 3b-2](2026-11-24-test-hygiene-suite-consolidation.md) 将非 `bin-as-module` 集成测试收拢为 7 个领域套件之后，`scripts/bin-as-module-baseline.json` 中仍留有 **49** 份 TUI 集成测试通过 `#[path = "../src/bin/tui.rs"] mod tui;` 引入 `[[bin]]` 目标源码。这导致 49 个集成测试二进制各自重复编译与执行约 2.3 万行 `tui` 源码及其 27 个内嵌单测模块，拖慢全量构建与 Windows/Linux CI，并迫使 `src/bin/tui.rs` 长期携带 `#![cfg_attr(test, allow(dead_code))]` 与 `#[allow(unused_imports)]` 掩蔽死代码。

## Decision

将 TUI 实现完整迁入库模块 `pony_agent_core::tui`，彻底移除集成测试对 `[[bin]]` 源码的 `#[path]` 内联：

1. **库模块归属与薄入口**：
   - 将 `crates/pony-agent-core/src/bin/tui.rs` 与 `crates/pony-agent-core/src/bin/tui/` 迁至 [`crates/pony-agent-core/src/tui.rs`](crates/pony-agent-core/src/tui.rs) 与 `crates/pony-agent-core/src/tui/`，在 [`crates/pony-agent-core/src/lib.rs`](crates/pony-agent-core/src/lib.rs) 公开 `pub mod tui;`。
   - [`crates/pony-agent-core/src/bin/tui.rs`](crates/pony-agent-core/src/bin/tui.rs) 保留为最小二进制入口，仅调用 `pony_agent_core::tui::run_main()`；`src/tui.rs` 移除 `#![cfg_attr(test, allow(dead_code))]` 与多余 `#[allow(unused_imports)]`，并删去 `cli_self_update_tests` 中未构建的死变体 `Route::NotFound`。
   - 对被 49 份集成测试消费的子模块 `theme`、`input` 与 `workspace_header_label_with_home` 提升为 `pub`，模块内 `pony_agent_core::` 引用统一切换为 `crate::`，并将 `tui/splash.rs` 内嵌图片相对路径调整为 `../../assets/splash-logo.png`。
2. **49 份测试与源码路径同步迁移**：
   - 将 [`scripts/bin-as-module-baseline.json`](scripts/bin-as-module-baseline.json) 登记的 **49** 份集成测试由 `#[path = "../src/bin/tui.rs"] mod tui;` 迁移为 `use pony_agent_core::tui;`，并将基线收缩至 **0 文件 / 0 处引用**（继续由 `contract:bin-imports` 门禁强锁零回归）。
   - 同步更新 `include_str!` / `CARGO_MANIFEST_DIR` 源码路径引用（`slash_commands_acceptance`、`tui_ctrl_o_global_shortcut`、`tui_streaming_input_acceptance`、`w3_sub_session_nav_test`、`tui_audit_ask_loop_regression`、`tui_session_resilience_l3b`、`tui_stop_static_wave60`、`tui/tests/splash_gradient_tests`、`runtime_config_suite/tui_audit_scenarios_regression`、`tools/tui-audit/checks.py` 与 `scripts/generate-splash-logo.py`）。
   - 为 [`tui_overlay_cancel_acceptance`](crates/pony-agent-core/tests/tui_overlay_cancel_acceptance.rs) 的 Python PTY 夹具补充独立会话（`start_new_session=True` + `TIOCSCTTY`），防止在拥有 24×80 控制终端的本地环境中继承宿主 `/dev/tty` 尺寸。
3. **三平台 Clippy 基线同步**：
   - 因 `src/bin/tui*` 迁入 `lib.rs` 成为库代码的一部分，`pony-agent-core` 与依赖它的 `src-tauri`（`pony-agent`）均会扫描该批模块：基于 Linux 实测生成最新 `platforms.linux` 分册（`scripts/clippy-baseline.json` 由 3,870 条违例降至 3,815 条；`scripts/clippy-baseline-src-tauri.json` 计入迁入的库模块后为 4,011 条），并将 `win32`/`darwin` 分册对应 `src/bin/tui*` 条目按等价路径映射同步至 `src/tui*`（`live <= base` 棘轮约束保持不变）。

## Consequences

- `crates/pony-agent-core/tests/` 的 `bin-as-module` 引用数量从 49 清零为 0，TUI 内嵌单测收敛到 `cargo test --lib` 单点运行一次，不再被 49 个集成测试二进制重复编译执行。
- 保留 [`crates/pony-agent-core/src/tui.rs`](crates/pony-agent-core/src/tui.rs) 顶部既有 `ponylint:file-size-exempt [until 2026-12-31]` 豁免声明，后续可独立按函数/子域继续拆分 `tui.rs` 而不受测试内联路径阻碍。

## Alternatives considered

- **仅抽出部分纯函数到 `src/tui/`、让 `TuiState` 与 `ui()` 留在 `src/bin/tui.rs`**：49 份集成测试中的绝大多数直接构造 `TuiState` 并调用 `ui()` 驱动 `TestBackend` 渲染，抽不走 `TuiState`/`ui()` 就无法清零 `bin-as-module-baseline.json`。
- **把 `src/bin/tui.rs` 留在 `src/bin/` 仅修改 `Cargo.toml` 声明多 path lib**：违反 Rust 惯例并会与 `bin/tui.rs` 共用路径导致门禁与源码扫描工具口径混乱；整体 `git mv` 入 `src/tui/`、`bin/tui.rs` 仅留薄入口更清晰且与 `cli` 拆分路线一致。
