# Agent Note: test-hygiene 3b-2 —— 非 bin-as-module 集成测试整合为 7 个领域套件并共享进程级锁

Status: implemented

## Problem

`crates/pony-agent-core/tests/` 原先共有 **263** 个顶层 `.rs` 文件，其中除了 49 个仍依赖 `#[path = "../src/bin/..."]` 的 `tui.rs` 测试留待 3b-3 拆库之外，其余 **214** 个非 `bin-as-module` 测试各自作为独立集成测试二进制编译与冷链接，成为 Windows CI（41m+）编译/链接膨胀与本地磁盘/构建时间飙升的主要瓶颈。

直接合套面临四类隐含耦合风险：
1. **全局进程状态互斥断裂**：43 份测试文件各自声明了文件私有 `static *_LOCK: Mutex<()>`，合并进同一二进制后变成多把互不相干的锁，导致 `HOME` / `XDG_*` / `PONY_AGENT_HOME` / `otel::STATE` / `turn_flow_registry` / `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT` 等全局状态在并发线程间互相踩踏；另有十余份依赖 `Once`（`isolate_process_data_dir` / `pin_promote_off`）或无锁写 `std::env` 的单测。
2. **宿主环境与全局 `OnceLock` 约束**：`wave25_capability_honesty_test`、`wave37_tool_surface`、`wave61_team_activation`、`pa110_teams_acceptance`、`team_drain_scope_test` 各自依赖 `SUBAGENT_ENGINE: OnceLock` 初始未置位或独占写入；`acp_proxy_autowire` 受宿主 `http_proxy` / `https_proxy` 继承污染；`pa108_search_acceptance::test_ac03` 在 POSIX 上 `temp_dir().join("..").join("..")` 逃逸到 `/` 触发全盘遍历；`permission_fail_closed_contract` 与 `wave23_governance_and_plan_sync_contract` 未绑定沙箱 `SessionStore`，会直连用户真实 `$HOME/.local/share/PonyAgent/sessions.db`。
3. **相对路径、子进程 `--exact` 与包属性**：`support/paths.rs`（14 处）、`cost_protocol_parity.rs` 的 `include_str!`（3 处）、`otel_trace_migration_test` 的自扫描守卫路径、`p1_multiprocess_writers` / `acp_session_lifecycle` 的 `current_exe() --exact` 过滤名，以及 3 份文件顶层 `#![cfg(unix)]` 在转为子模块后路径或属性层级发生变化。
4. **基线门禁同步**：`scripts/file-size-baseline.json` 有 7 条存量规则绑定旧的顶层测试文件路径。

## Decision

将 **204** 份非 `bin-as-module`、非 CLI/CI 锚定、非独占 `OnceLock` 的顶层集成测试文件按领域整合为 **7** 个集成测试套件（顶层测试目标总计从 263 降至 66，减少 ~75% 独立测试二进制链接数）：

- `acp_suite`（19 份）
- `observability_suite`（33 份）
- `session_storage_suite`（35 份）
- `stop_process_suite`（30 份；将 `process_group_reap_test`、`range_quiescence_test`、`run_exit_code_class_test` 的 `#![cfg(unix)]` 转为 `main.rs` 模块级 `#[cfg(unix)] mod ...;`）
- `tools_sandbox_suite`（32 份）
- `runtime_config_suite`（48 份）
- `agent_team_goal_suite`（7 份）

同时保持其余 **59** 个顶层文件独立不动：49 个 `bin-as-module` 文件与 2 个 `include_str!("../src/bin/tui.rs")` 文件（留待 3b-3 解耦）、3 个脚本/CI 显式 `--test` 锚定文件（`conformance`、`ts_contract_snapshot`、`tui_startup_input_guard`），以及 5 个独占 `SUBAGENT_ENGINE: OnceLock` 约束的文件（`wave25_capability_honesty_test`、`wave37_tool_surface`、`wave61_team_activation`、`pa110_teams_acceptance`、`team_drain_scope_test`）。

配套工程收口：
1. 新增轻量共享辅助 [`tests/support/env_lock.rs`](crates/pony-agent-core/tests/support/env_lock.rs) 与 [`tests/support/test_name.rs`](crates/pony-agent-core/tests/support/test_name.rs)（按套件按需引入，保持 `-D warnings` 下零死代码告警）。
2. 将 43 份文件私有 env/OTel/persist `Mutex` 与十余份无锁 `set_var` / `isolate_process_data_dir` / `pin_promote_off` 用例统一改为持 `crate::env_lock::lock()` + RAII `Drop` 还原。
3. 修复同进程暴露的既有测试卫生问题：`acp_proxy_autowire::TempHome` 临时剥离宿主 `*_proxy` 变量、`pa108_search_acceptance::test_ac03` 改为三层嵌套受控沙箱目录不逃逸到 `/`、`permission_fail_closed_contract` 与 `wave23_governance_and_plan_sync_contract` 显式绑定临时 `FileSessionBackend`。
4. 更新 [`scripts/file-size-baseline.json`](scripts/file-size-baseline.json) 中的 7 条迁移路径。

## Verification

- 全量 7 个新领域套件 + 10 个独立非 bin 目标在 `RUSTFLAGS="-D warnings"` 下实跑全绿（0 failed）：
  - `acp_suite`: `143 passed`
  - `agent_team_goal_suite`: `20 passed`
  - `observability_suite`: `183 passed`（1 ignored）
  - `runtime_config_suite`: `277 passed`
  - `session_storage_suite`: `170 passed`（4 ignored）
  - `stop_process_suite`: `201 passed`（1 ignored）
  - `tools_sandbox_suite`: `193 passed`
  - 独立非 bin 测试（`conformance` 28、`ts_contract_snapshot` 1、`tui_startup_input_guard` 2、`tui_session_resilience_l3b` 4、`tui_stop_static_wave60` 3、`pa110_teams_acceptance` 6、`team_drain_scope_test` 1、`wave25_capability_honesty_test` 4、`wave37_tool_surface` 9、`wave61_team_activation` 16）全部 passed
- `RUSTFLAGS="-D warnings" CARGO_TARGET_DIR=target-test bash scripts/cargo-lane.sh test --no-run --manifest-path src-tauri/Cargo.toml -p pony-agent-core`：全部 66 个测试目标与库/二进制 0 warning 编译通过
- `npm run size:check`、`npm run contract:bin-imports`（49/49）、`npm run contract:bin-imports:selftest`、`npm run rust:lint`、`npm run cargo:check:shared`、`npm run version:check`、`npm run contract:check`、`npm run contract:cli`、`npm run ext:check`、`npm run adr:check`、`npm run backlog:gate`、`npm run secrets:check:all`、`GATE_BASE=origin/main node scripts/observability/gate.mjs`、`audit-diff.sh --base origin/main` 全部退出 0

## Alternatives considered

- **不区分领域直接合进单一 `integration_suite`**：单二进制体积过大且 `test_ac`/`env_lock` 串行队列过长，本地精准复现某一领域（例如 `--test observability_suite` 或 `--test acp_suite`）也丧失粒度。
- **修改生产代码 `SUBAGENT_ENGINE` 为 `RwLock` 以把 wave25/37/61/pa110 一起合并**：修改核心运行时并发语义超出 3b-2 测试目录整合边界，保持该 5 个文件独立编译是最安全、最小侵入的做法。
