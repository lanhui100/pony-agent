//! pony: Terminal TUI entrypoint.
//!
//! 本体逻辑归属库模块 `pony_agent_core::tui`，本文件仅保留最小二进制启动入口。

fn main() {
    pony_agent_core::tui::run_main();
}
