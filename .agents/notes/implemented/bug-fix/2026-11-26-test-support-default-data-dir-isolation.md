# Agent Note: test-support 构建下默认数据目录自动隔离（根治集成 / bin 测试误读生产会话库）

Status: implemented

## Problem

`crates/pony-storage/src/sqlite_session/sqlite_paths.rs`（`default_sqlite_path()`）、
`crates/pony-storage/src/session/store.rs`（`default_storage_path()`）与
`crates/pony-observability/src/frontend_diagnostics.rs`（`default_frontend_diagnostics_sqlite_path()`）
三处默认数据路径的测试隔离原本只有 `#[cfg(test)]` 一道门。

Rust 编译集成测试（`tests/*.rs`）与 `[[bin]]` 测试时，依赖 crate 是以 `#[cfg(not(test))]`
编译的——`cfg(test)` 只在被测 crate 自己的 `--lib` 单元测试里成立。于是任何集成测试构造
`HostControlPlaneBuilder::new().build()` / `SessionStore::new()` 而未显式注入
`SessionStore::memory_only()` 或 `PONY_AGENT_DATA_DIR` 时，会直接命中宿主真实
`~/.local/share/PonyAgent/sessions.db` 与 `session-logs/`，触发全量日志 fold 与投影回填：
实测 GB 级 RSS + 11GiB Swap 颠簸，把整机拖垮（账本 `backlog:B1795589917533` →
`B1795681671813`）。

既有 `PONY_AGENT_DATA_DIR` 旁路（`.agents/notes/implemented/bug-fix/2026-11-11-windows-sweep-tui-tests-and-backlog-cli-entrypoint.md`）
只在测试**主动设置**时生效，缺「未显式设置时也绝不回落生产目录」的底座防线。

## Decision

1. **单点判决放 `pony-protocol`**：`crates/pony-protocol/src/sqlite_paths.rs` 声明
   `test-support` feature（`[features] default = []` + `test-support = []`），并提供：
   - `data_dir_root() -> Option<PathBuf>`：**唯一对外判决口**，顺序为
     `data_dir_override()`（`PONY_AGENT_DATA_DIR`）→ `test_support_sandbox_root()` → `None`；
   - `test_support_sandbox_root() -> Option<PathBuf>`（私有，编译期分叉在函数体内）：
     `cfg(all(not(test), feature = "test-support"))` 时 `Some(fallback)`，其余恒 `None`；
   - `test_support_fallback_data_dir()`（仅 `cfg(all(not(test), feature = "test-support"))`）：
     进程级 `OnceLock` 固定 `std::env::temp_dir()/pony-agent-test-support-<pid>-<nanos>`。
2. **三处默认路径只把 `data_dir_override()` 换成 `data_dir_root()`**：`default_sqlite_path()`、
   `default_storage_path()`、`default_frontend_diagnostics_sqlite_path()` 各改一行，其余判定链
   （`dirs::data_local_dir` → `home_dir` → cwd）原样保留。生产构建里 `data_dir_root()` 与
   `data_dir_override()` 逐字等价，零行为漂移；`default_storage_path()` 因此净增 0 行。
3. **feature 的启用方是 `pony-test-support`**（各 crate 的 dev-dependency，只在测试构建入图）：
   `crates/pony-test-support/Cargo.toml` 增 `pony-protocol = { path = "...", features = ["test-support"] }`。
   Cargo 的 feature 统一把该特性带给同一次构建里的 `pony-protocol` / `pony-storage` /
   `pony-observability`，从而做到「测试构建开、生产构建关」，且不需要任何 crate 自引用。
4. **防回归覆盖**：新增 `crates/pony-agent-core/tests/session_storage_suite/test_support_data_dir_isolation.rs`
   （注册进该 suite 的 `main.rs`），锁两条契约：未设 `PONY_AGENT_DATA_DIR` 时两处默认路径一律落
   `pony-agent-test-support-*` 沙箱且同进程内稳定、绝不落入诱饵 `XDG_DATA_HOME`；显式设置时立即优先。

`#[cfg(test)]`（`--lib` 单元测试每次唯一临时路径）与
`cfg(all(not(test), not(feature = "test-support")))`（生产二进制）行为逐字不变。

## Alternatives considered

- **照 2026-11-25 备份分支原样移植（feature 声明在被测 crate 自己身上 + `[dev-dependencies]` 自引用启用）**：
  语义等价但工程上不可行——`2026-11-25-build-test-layering-and-workspace-crate-split` 已经删掉
  `pony-agent-core` 的自引用 `test-support` dev-dependency（同一 crate 二次编译），再加回去就是
  推翻该 ADR 的实测性能收益。改为「feature 声明在 `pony-protocol`、启用方是 `pony-test-support`」，
  既拿到同一编译期门，又不需要自引用。
- **只逐个测试手写 `PONY_AGENT_DATA_DIR` / `SessionStore::memory_only()`，不动默认路径函数**：
  依赖每位作者自觉注入，容易遗漏；`tests/*_suite` 大量经由 `#[path]` 复用的场景下漏一处就复发
  同一事故。宁可把防线放在路径解析的单一出口。
- **`test-support` 下让 `default_*_path()` 每次返回不同随机目录（复用 `unique_test_sqlite_path()` 口径）**：
  同进程内「先写后读同一临时库」的集成测试会读不到前一次写入。`OnceLock` 既隔离生产库又保持
  同进程内多次调用一致，且不妨碍测试用 `PONY_AGENT_DATA_DIR` 逐次覆盖（旁路优先于 fallback）。
- **让 `HostControlPlaneBuilder::new()` 在 `test-support` 下直接改用 `SessionStore::memory_only()`**：
  会静默改变大量 `HostControlPlaneBuilder` + SQLite/Ask/Event 持久化集成测试的语义；只收口默认路径
  解析既保留真实 SQLite 链路测试能力，又不触碰生产数据。
- **在 `pony-storage` / `pony-observability` 各声明一份同义 feature 与 fallback 函数**：
  两份 `OnceLock` 会让同一进程的会话库与诊断库落进**两个不同**沙箱根，且两套判据必然漂移。
  单点判决（P9：一份判据一份实现）是本方案的核心取舍。

## Consequences

- 任何链接 `pony-test-support`（dev-dependency）的集成 / `[[bin]]` 测试，在未设置
  `PONY_AGENT_DATA_DIR` 时只会读写系统临时目录下的进程级干净沙箱，不再有误加载生产
  `sessions.db` 导致 OOM / Swap 颠簸的风险。
- 生产构建（不启用 `test-support`）与依赖 `PONY_AGENT_DATA_DIR` 的既有集成测试、`perf_probe` /
  `perf_lazy` example 行为保持不变。
- 测试进程会按需在 `std::env::temp_dir()` 下生成 `pony-agent-test-support-<pid>-<nanos>`
  临时目录（仅在未注入 `memory_only` 且未设 `PONY_AGENT_DATA_DIR` 时），体积可忽略。
- `pony-acp` / `pony-storage` / `pony-observability` 自身没有集成测试目录，其测试面走 `cfg(test)`
  分支，不受本决定影响；将来若新增，须先加 `pony-test-support` dev-dependency 才继承本隔离。
- 残留风险：feature 统一是「一次构建一个特性集」，因此 `cargo build --workspace`（把
  `pony-test-support` 当作 workspace 成员一起构建）会让该次构建里的二进制也带上沙箱兜底；
  生产发布路径（`npm run tauri build` / `cargo build -p pony-cli` 等单包构建）不含 dev-dependency，
  不受影响。这条靠 review 守住，无机械门。
- 观测到的既有失败（**与本决定无关**，基线 `origin/main` 复现）：`cargo test -p pony-agent-core
  --test runtime_suite` 全量跑时 `team_drain_scope_test` / `team_state_observability_test` /
  `teammate_e2e_communication_test` / `wave25_capability_honesty_test` 四例失败、单独跑全绿——
  根因是 `subagent_runtime::SUBAGENT_ENGINE` 的进程级 `OnceLock` 被多个测试文件抢注（首个 set 者
  生效、其余 `set` 返回 Err 被 `let _ =` 丢弃）。已单独复跑四个用例全部通过，故本决定的验证证据
  以「单文件定向跑 + 基线对照」为准。
