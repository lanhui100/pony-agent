# Agent Note: test-support 下默认数据目录自动隔离（根治集成/bin 测试误读生产会话库）

Status: implemented

## Problem

在 `crates/pony-agent-core/src/agent/sqlite_session/sqlite_paths.rs`、`crates/pony-agent-core/src/agent/frontend_diagnostics.rs` 与 `crates/pony-agent-core/src/agent/session/store.rs` 中，测试路径隔离原本仅通过 `#[cfg(test)]` 门控。然而 Rust 编译集成测试（`tests/*.rs`）与 `[[bin]]` 测试时，依赖 crate `pony_agent_core` 以 `#[cfg(not(test))]` 编译（仅通过 `[dev-dependencies]` 启用 `feature = "test-support"`）。

一旦任何集成测试或 `#[path = "../src/bin/tui.rs"]` 嵌套测试（如 `tui_task_visibility` 中的 `ask_lifecycle_settle_tests::report_control_request_error_without_handle_stays_neutral`）构造 `HostControlPlaneBuilder::new().build()` 而未显式注入 `runtime`/`SessionStore::memory_only()` 或覆盖 `PONY_AGENT_DATA_DIR`，`SessionStore::new()` 会直接命中宿主真实 `~/.local/share/PonyAgent/sessions.db` 与 `session-logs/`，触发全量日志 fold 与投影回填，导致测试卡死、内存暴涨至数 GiB 并拖垮机器 I/O（待办 `B1795589917533`）。

既有 `PONY_AGENT_DATA_DIR` 旁路（见 `.agents/notes/implemented/bug-fix/2026-11-11-windows-sweep-tui-tests-and-backlog-cli-entrypoint.md`）只在测试主动设置时生效，缺少「未显式设置时也绝不回落生产目录」的底座防线；同时 `SessionStore::memory_only()` 已在 `.agents/notes/implemented/architecture/2026-11-11-multi-turn-tool-context-pruning-dsh.md` 中通过 `test-support` 门控。

## Decision

1. **底座默认隔离（`#[cfg(all(not(test), feature = "test-support"))]`）**：
   - 在 `crates/pony-agent-core/src/agent/sqlite_session/sqlite_paths.rs` 新增进程级 `OnceLock<PathBuf>` 临时沙箱根 `test_support_fallback_data_dir()`（`std::env::temp_dir()/pony-agent-test-support-<pid>-<nanos>`），并在 `sqlite_session.rs` 对内 `pub(crate)` 导出。
   - 在 `default_sqlite_path()`、`default_frontend_diagnostics_sqlite_path()`（`frontend_diagnostics.rs`）与 `default_storage_path()`（`session/store.rs`）中增加 `#[cfg(all(not(test), feature = "test-support"))]` 分支：动态优先读取 `data_dir_override()`（`PONY_AGENT_DATA_DIR`），未设置时一律回落 `test_support_fallback_data_dir().join("PonyAgent")`，彻底切断对 `dirs::data_local_dir()` / `dirs::home_dir()` 的生产路径回退。
   - 保持 `#[cfg(test)]`（`--lib` 单元测试每次唯一路径）与 `#[cfg(all(not(test), not(feature = "test-support")))]`（生产二进制）行为逐字不变。
2. **存量未隔离测试切到 `SessionStore::memory_only()`**：
   - 为不需要真实 SQLite 落盘的 6 处测试（`ask_lifecycle_settle_tests.rs`、`wave65_r3_approval_stale_notice_tests.rs`、`ask_pending_contract.rs`、`permission_fail_closed_contract.rs`、`stop_escalation_wave60.rs`、`wave23_governance_and_plan_sync_contract.rs`）显式注入 `SessionStore::memory_only()`，消除测试期不必要的 SQLite 建表与临时文件 I/O。
3. **防回归覆盖**：
   - 新增 `crates/pony-agent-core/tests/test_support_data_dir_isolation.rs`，锁定「未设 `PONY_AGENT_DATA_DIR` 时绝不落入 `XDG_DATA_HOME`/生产目录且 `HostControlPlaneBuilder::new().build()` 亚秒完成」以及「显式设置 `PONY_AGENT_DATA_DIR` 立即优先」两条契约。

## Alternatives considered

1. **仅逐个测试手写 `XDG_DATA_HOME` / `SessionStore::memory_only()`，不动底座路径函数**：落选。依赖每位作者自觉注入容易遗漏，且在 `tests/tui_*.rs` 大量通过 `#[path = "../src/bin/tui.rs"]` 复用内联测试的场景下，只要漏掉一处就能再次触发 GB 级生产库加载事故。
2. **在 `test-support` 分支下像 `unique_test_sqlite_path()` 一样每次调用返回不同随机目录**：落选。同进程内通过 `SessionStore::new()` 与 `session_persistence_evidence()`、或 `goal_store_path()` 与 `spool_path()` 跨调用读写同一临时库的集成测试会读不到前一次写入，而进程级 `OnceLock` 既隔离生产库又保持同进程内多次调用路径一致，且不阻碍测试通过 `PONY_AGENT_DATA_DIR` 动态覆盖。
3. **让 `HostControlPlaneBuilder::new()` 默认在 `test-support` 下改为 `SessionStore::memory_only()`**：落选。会静默改变大量测试 `HostControlPlaneBuilder` 与 SQLite/Ask/Event 持久化的集成测试语义，而只收口默认路径解析既能保留真实 SQLite 链路测试能力，又保证不会触碰生产数据。

## Consequences

- 任何链接 `[dev-dependencies]`（自动启用 `test-support`）的集成测试或 bin 测试，在未设置 `PONY_AGENT_DATA_DIR` 时都只会读写系统临时目录下的进程级干净沙箱，不再出现意外加载生产 `sessions.db` 导致 OOM / Swap 颠簸的风险。
- 生产构建（不启用 `test-support`）以及依赖 `PONY_AGENT_DATA_DIR` 的既有集成测试与 `perf_probe` / `perf_lazy` example 行为保持不变。
- `test-support` 测试进程会在 `std::env::temp_dir()` 下按需生成 `pony-agent-test-support-<pid>-<nanos>` 临时目录（仅在未注入 `memory_only` 且未设 `PONY_AGENT_DATA_DIR` 时创建），体积可忽略。
