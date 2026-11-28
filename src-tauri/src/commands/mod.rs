//! Tauri 命令面按业务域拆分（B1795831358336 Phase 1）。
//!
//! 拆分契约：
//! - **命令名 / 参数签名 / 返回形状逐字不变**：前端 `safeInvoke("...")` 与
//!   `src-tauri/tests/tauri_command_surface.rs` 的冻结清单都依赖这一点；
//! - 每个子模块只装同一业务域的命令，模块内自包含（导入、视图结构体、域内 helper 同处一文件），
//!   降低多 Agent 并行改同一文件时的合并冲突；
//! - `lib.rs` 只保留装配（`run()`、退出 flush、`generate_handler!` 注册表）。

pub(crate) mod acp;
pub(crate) mod ask;
pub(crate) mod capability;
pub(crate) mod diagnostics;
pub(crate) mod goal;
pub(crate) mod history;
pub(crate) mod host;
pub(crate) mod plan;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod team;
pub(crate) mod turn;
pub(crate) mod workspace;
