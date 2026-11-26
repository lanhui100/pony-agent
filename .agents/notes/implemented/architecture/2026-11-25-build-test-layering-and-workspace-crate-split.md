# Agent Note: 编译测试分层、防主机卡死护栏与 Workspace 多 Crate 架构落地

Status: implemented

## Context

2026-11-25 实测主工作机为 AMD Ryzen 7 5700U（8 物理核 / 16 线程 / 30 GiB RAM / 31 GiB Swap）。在引入 `8-job` 并发与全局构建锁之后，工程仍受困于四类结构性瓶颈：

1. **单 Crate 巨石触及单核编译天花板**：`crates/pony-agent-core` 集成了全部协议、存储、Provider、工具、ACP、可观测性以及 `tui`（约 3.2 万行及 `ratatui`/`ratatui-image`/`crossterm`）和 `cli` 二进制，热编长期受限于单 `rustc` 后端串行天花板（128–151s）。
2. **集成测试二进制爆炸与自引用二次编译**：`crates/pony-agent-core/tests/` 下存在 266 个独立顶层 `.rs` 文件（266 次独立链接），且 `pony-agent-core` 在 `[dev-dependencies]` 自引用 `features = ["test-support"]`，测试构建期间对核心库触发额外编译。
3. **缺少高速链接与分层测试闭环**：Linux 默认走单线程 GNU `ld` 且未接入 `cargo-nextest` 分组调度与 L0–L3 分层命令。
4. **防主机卡死护栏存在盲区**：失控单测进程无内核级 CPU/内存/IO 边界及 `RUST_TEST_THREADS` 限流，曾导致单进程 8.5 GiB RSS 触发整机 Swap 颠簸并长期锁死 `/tmp/pony-agent-build.lock`。

相关上游决策：
- `.agents/notes/implemented/process/2026-11-20-build-engineering-four-fronts.md`
- `.agents/notes/implemented/process/2026-11-20-ci-scope-split.md`
- `.agents/notes/implemented/architecture/2026-11-16-native-range-owner-systemd-scope.md`

## Decision

在现有 Monorepo Cargo Workspace 内分两阶段完成构建测试分层、主机防卡死护栏与严格无环多 Crate 架构拆解：

### Phase 1：运行护栏、链接测试加速与集成套件收口

1. **防主机卡死三重护栏（`B1795589917594`，`scripts/cargo-lane.sh`）**：
   - 在 Linux 探测 `systemd-run --user --scope`，默认注入 `CPUQuota=600%`、`MemoryHigh=14G`、`MemoryMax=18G`（支持环境变量覆盖；无 user-systemd 权限时安全降级并在 `on|1` 显式开关下 fail-closed 退出码 2）。
   - 叠加 `ionice -c 3` 空闲级磁盘 IO 调度；将 `$HOME/.cargo/bin` 置于 `/usr/local/bin` 之前（同时不遮蔽前置测试桩目录），统一 toolchain 解析。
   - 在 `test` / `nextest` 路径默认注入 `RUST_TEST_THREADS=4` 与 `timeout`（默认 600s，置于构建锁内部防止持锁无限挂死）。
2. **高速链接器、Nextest 与 L0–L3 测试分层（`B1795615060280`）**：
   - `scripts/cargo-lane.sh` 与 `scripts/build-accel-check.sh` 接入 `mold -run` 链接通道；`.config/nextest.toml` 建立 `default` / `quick` / `ci` 三档 profile 与 `heavy-pty` / `heavy-e2e`（`max-threads = 2`）并发组。
   - `package.json` 与 `scripts/run-test-layer.mjs` 提供 `test:l0`（`check:fast`）、`test:l1`（全 workspace `--lib` nextest `quick` + `typecheck`）、`test:l2`（`test:unit` + `rust:lint` 双基线 + `--lib` `default`）与 `test:l3`（全量 `nextest --profile ci` + `test:unit` + `build`），并在缺失 `cargo-nextest` 时自动剥离 `--profile` 降级回 `cargo test`。
3. **集成测试按域收口（`B1795615060223`）**：
   - `crates/pony-agent-core/tests/` 收口为 5 个域测试二进制：`acp_suite`、`observability_suite`、`runtime_suite`、`session_storage_suite`、`tools_stop_suite`（外加独立 crate 内的 `tui_suite` 与 `cli_suite`），共将顶层二进制数压至 ≤ 6 个每 crate。
4. **独立 `crates/pony-test-support`（`B1795615060339`）**：
   - 新增独立 workspace crate `crates/pony-test-support`（依赖 `pony-storage`，提供 `memory_only_session_store()` 等共享夹具），移除 `crates/pony-agent-core/Cargo.toml` 自引用 `test-support` dev-dependency。

### Phase 2：Workspace 无环多 Crate 拆分与精细化 CI 分流

1. **TUI 与 CLI 独立 Crate（`B1795615060398`）**：
   - 新增 `crates/pony-tui`（二进制 `pony`）与 `crates/pony-cli`（二进制 `ponycli`）；`pony-agent-core` 与 `src-tauri`（`pony-agent`）彻底卸载 `ratatui`、`ratatui-image`、`crossterm` 等终端 UI 依赖。
   - 更新 `package.json`（`cli:build`）、`tools/tui-audit/checks.py`、`tests/acceptance/l2at-tui-*.sh` 与 `scripts/file-size-baseline.json`。
2. **核心库按 6 层无环 DAG 拆分（`B1795615060457`）**：
   - `pony-protocol`（Layer 0，协议/常量/事件/类型/hooks 契约）
   - `pony-observability`（Layer 1，依赖 `pony-protocol`：`otel`、`ponysentry`、`frontend_diagnostics`、`usage_ledger`）
   - `pony-provider` 与 `pony-storage`（Layer 2，并行依赖 `pony-protocol` + `pony-observability`）
   - `pony-tools`（Layer 3，依赖前三层，通过 `OrchestrationToolBridge` 与 `host_hooks` 解耦反向编排调用）
   - `pony-acp`（Layer 4，依赖 `pony-tools` 及下层）
   - `pony-agent-core`（Layer 5，核心 runtime/planner/control_plane/turn_flow，通过 `pub use` 维持既有 `pony_agent_core::agent::*` 与 `pony_agent_core::acp` 兼容导出面，并裁剪 `anydoc`/`encoding_rs`/`globset`/`ignore`/`keyring`/`winreg` 冗余直接依赖）
   - `pony-tui`、`pony-cli`、`pony-agent`（Layer 6，并行消费 `pony-agent-core`）
   - 同步更新 `docs/observability.yaml` 的 `tracing`/`issues`/`http_client` 锚点至 `crates/pony-observability/src/`、`scripts/clippy-baseline*.json`（三平台移动路径重映射及 Linux 基线同步）与 `docs/architecture/extension-points.md`。
3. **CI 细粒度变更面分流（`B1795615060519`）**：
   - 升级 `scripts/ci-scope.mjs`、`scripts/ci-scope.spec.sh` 与 `.github/workflows/ci.yml`，输出 `needs_rust`、`needs_frontend`、`needs_core`、`needs_tui`、`needs_cli`、`needs_tauri` 与 `scope_reason`：按 workspace 依赖传递闭包精确决定编译/测试面，根配置/脚本/CI/未知路径/`no-base`/错误一律 fail-safe 回落全量 `true`。

## Alternatives considered

- **A. 拆分为多个独立 Git 仓库（Multi-repo）**：否决。前端、`src-tauri`、TUI、CLI 与核心库共享 `contract:check`、`version:check` 同步契约与单 PR 原子验证，拆仓会放大版本锁定与联调成本；编译慢的根因在于单 crate 体积过大，Workspace 内部拆分即可彻底解决。
- **B. 仅提高 `max_jobs_per_lane`（如 12–16）堆并发**：否决。先前标定表明 12 job 对单 crate 热编毫无收益，仅推高峰值内存并加剧主机 Swap 风险。
- **C. 跳过 Phase 1 直接大爆炸重写核心库多 Crate**：否决。Phase 1 先提供 cgroup/ionice 护栏、`mold`、`nextest` 与测试收口安全网，使 Phase 2 每一步切分都具备快速、可定位的回归护栏。
- **D. 全局关闭 `incremental` 换取 `sccache`**：否决。叶子与领域 crate 的日常小改高度依赖 `incremental` 编译，保留 `PONY_BUILD_SCCACHE=auto` 仅在冷编或 `--release` 启用。

## Consequences

1. **Workspace 拓扑可机械验真**：`cargo metadata` 验证内部依赖为严格 6 层无环 DAG；`pony-agent-core` 与 `src-tauri` 依赖树零 `ratatui`/`crossterm`；`RUSTFLAGS="-D warnings"` 全工作区 `--all-targets` 编译零告警通过。
2. **叶子 Crate 增量检查压缩至 ≤ 12s**：`target-check` 槽位实测（`jobs=8, incremental=1`）各模块增量热检耗时：
   - `pony-provider`：2.58s
   - `pony-storage`：5.53s
   - `pony-cli`：5.78s
   - `pony-tui`：6.06s
   - `pony-observability`：7.63s
   - `pony-protocol`：8.27s
   - `pony-agent-core`：11.12s（峰值 rustc RSS 564 MB）
3. **测试隔离强化**：补齐了 `pony-tui`、`control_plane`、`planner`、`subagent_runtime`、`runtime` 与 `tools_stop_suite` 中多处未隔离 `SessionStore::memory_only()` / `TempGoalStore` / `open_in_memory_ask_store()` 的单测，并修复 `acp_delegate_p0p2_contract::t_vis_02` 的 `env_lock` 重入死锁与 `host_hooks::is_subagent_runtime_wired()` 未注册探针时的 fail-open 缺口（改为 `false`），消除 `nextest` 每用例独立进程下误读真实宿主 `sessions.db` 或 `goal_store.json` 的挂死与污染隐患。
4. **绑定待办闭环（批次 `20261125T135740Z-dev-infra-e843` + `20261126T011813Z-general-af06`）**：`B1795589917594`、`B1795615060223`、`B1795615060280`、`B1795615060339`、`B1795615060398`、`B1795615060457`、`B1795615060519` 共 7 项全部状态闭环并附带实测验证收据。
5. **非 Linux 分册提示**：`scripts/clippy-baseline.json` 与 `scripts/clippy-baseline-src-tauri.json` 的 `win32`/`darwin` 分册已按文件搬迁路径做确定性映射，后续 Windows/macOS CI 首跑若因跨平台分支拆分产生精确计数微调，可由对应 runner `--update-baseline` 校验对齐。
