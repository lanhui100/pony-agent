# Agent Note: 编译测试分层、防主机卡死护栏与 Workspace 多 Crate 演进方案

Status: proposed

## Problem

2026-11-25 实测主工作机为 AMD Ryzen 7 5700U（8 物理核 / 16 线程 / 30 GiB RAM / 31 GiB Swap，其中 Swap 已用 8.6 GiB）。尽管前期已完成基础构建治理，当前工程仍存在以下结构性编译慢、测试膨胀与主机卡死隐患：

1. **单 Crate 巨石触及单核编译天花板**：`crates/pony-agent-core` 集中了 253,088 行生产代码与 119,976 行测试代码（含 `src/bin/tui.rs` + `src/bin/tui/` 约 3.2 万行及 `ratatui`/`image`/`crossterm` 等终端 UI 依赖）。即使在 8–12 并发下，单个 crate 仍受限于单 `rustc` 进程后端代码生成，全目标热编时间长期停留在 128–151s。
2. **集成测试二进制爆炸与自引用二次编译**：`crates/pony-agent-core/tests/` 下存在 266 个独立顶层 `.rs` 文件，导致 Cargo 为每个文件独立编译和链接（实测 `target-test` 含 503 个可执行文件、占盘 24 GiB）；同时 `crates/pony-agent-core/Cargo.toml` 在 `[dev-dependencies]` 中自引用 `pony-agent-core = { path = ".", features = ["test-support"] }`，测试期因 feature 集合不同会迫使核心库额外编译一遍。
3. **快速链接与测试分层工具缺失**：`bash scripts/build-accel-check.sh` 实测本机缺少 `mold`、`ld.lld`、`clang` 与 `cargo-nextest`，Linux 默认走单线程 GNU `ld`；`package.json` 仅粗分 `check:fast`、`cargo:check:shared` 与全量 `cargo:test:shared`，缺少面向日常迭代与验收的 L0–L3 测试分层及并发组控制。
4. **防主机卡死护栏存在盲区**：既有 `scripts/cargo-lane.sh` 已实现 8-job 预算、跨输出目录全局锁、load 熔断与 `test` 180s 超时，但缺乏内核级 CPU/内存/IO 隔离及测试运行期并发线程限流。2026-11-25 实证（见 backlog `B1795589917594`）：单个失控测试进程吃满 8.5 GiB RSS 并引发 11 GiB Swap 颠簸，拖垮整机 I/O 并长期占用 `/tmp/pony-agent-build.lock`。

本提案承接并推进以下已记录决策：
- `.agents/notes/implemented/process/2026-11-20-build-engineering-four-fronts.md`（标定 4/8/12 job 热编卡在 128–151s，并将 `mold` 默认策略与拆分 `pony-agent-core` 暂缓至本轮结构演进）
- `.agents/notes/implemented/process/2026-11-20-ci-scope-split.md`（当前 `ci-scope.mjs` 仅区分纯文档与全量代码面）
- `.agents/notes/implemented/process/2026-11-07-tui-decomposition-and-size-gate.md`
- `.agents/notes/implemented/process/2026-11-08-tools-router-module-split.md`
- `.agents/notes/implemented/process/2026-11-09-cli-release-platform-matrix.md`
- `.agents/notes/implemented/architecture/2026-11-16-native-range-owner-systemd-scope.md`（注：部分运行环境无 user-systemd 权限，cgroup 护栏须探测并在缺失时安全降级）

## Proposal

总体原则：**不拆分为多个独立 Git 仓库（Multi-repo），而是在现有 Monorepo Cargo Workspace 内分两阶段渐进演进**——先通过无业务架构风险的 Phase 1 收割链接/测试/防卡死红利，再在 Phase 2 拆分 Workspace Crate 打破单 crate 编译天花板。

### Phase 1：零业务重构的提速与主机防卡死护栏

1. **防主机卡死三重硬护栏（`B1795589917594`）**：
   - 在 `scripts/cargo-lane.sh` 外层优先探测 Linux `systemd-run --user --scope`，为构建与测试施加内核级 cgroup v2 资源边界（例如 `CPUQuota=600%`、`MemoryHigh=14G`、`MemoryMax=18G`、`IOWeight=50`，参数支持环境变量覆盖）；在无 user-systemd 权限或非 Linux 平台安全降级为 `ulimit` 软约束与可观测告警，避免假绿或无法运行。
   - 叠加 `ionice -c 3`（空闲级磁盘 IO 调度），避免链接与测试写盘抢占交互式会话 IO。
   - 在 `test` 路径统一注入可配置的 `RUST_TEST_THREADS`（默认 4–6），防止 16 逻辑核测试并发叠加 Tokio 多线程引发线程与内存爆炸。
2. **集成测试分组收口（`B1795615060223`）**：
   - 将 `crates/pony-agent-core/tests/` 下 266 个顶层独立 `.rs` 文件按领域合并为 4–6 个测试入口（如 `tests/tui/main.rs`、`tests/acp/main.rs`、`tests/session/main.rs`、`tests/runtime/main.rs`、`tests/acceptance/main.rs`），内部以 `mod` 聚合原有测试模块，将全量集成测试链接次数从 266 次压降至 ≤ 6 次，同步更新 `.github/workflows/ci.yml` 及引用 `--test <name>` 的脚本。
3. **高速链接器 `mold` + `cargo-nextest` + L0–L3 测试分层（`B1795615060280`）**：
   - 评估并在 Linux 目标启用 `mold`（或 `ld.lld`）替代单线程 GNU `ld`，配合 `scripts/build-accel-check.sh` 与 `scripts/build-bench.sh` 完成量化标定，避免频繁切换 `RUSTFLAGS` 造成增量缓存失效。
   - 引入 `cargo-nextest` 与 `.config/nextest.toml`，提供 `quick`、`default`、`ci` 三套 profile，并为 PTY/子进程/重内存测试配置 `test-groups`（`max-threads = 2`），在无 `nextest` 环境保留 `cargo test` 降级路径。
   - 在 `package.json` 建立清晰的 L0–L3 分层命令契约：
     - **L0（秒级）**：`check:fast`（`--lib`）+ 单文件前端/Rust 单测。
     - **L1（10–30s）**：改动 crate 的 `--lib` 单元测试 + 轻量契约检查。
     - **L2（1–2min）**：对应域集成测试组 + 前端 `test:unit` + `rust:lint` 棘轮门。
     - **L3（全量/发布前）**：全量集成回归 + PTY 跨平台验证 + Playwright E2E + release 构建。
4. **消除 `test-support` 自引用二次编译（`B1795615060339`）**：
   - 将 `crates/pony-agent-core` 在 `[dev-dependencies]` 中自引用 `features = ["test-support"]` 的测试夹具与内存桩剥离至独立 workspace crate `crates/pony-test-support`（或收口 feature 边界），维持生产二进制零测试桩及 `test_support_data_dir_isolation` 临时沙箱隔离契约不变。

### Phase 2：Workspace 内多 Crate 拆分与精细化 CI

1. **第一刀：剥离终端二进制 Crate `pony-tui` 与 `pony-cli`（`B1795615060398`）**：
   - 新增 `crates/pony-tui`（迁出 `src/bin/tui.rs` + `src/bin/tui/` 约 3.2 万行及 `ratatui`/`ratatui-image`/`image`/`crossterm` 依赖）与 `crates/pony-cli`（迁出 `src/bin/cli.rs` + `src/bin/cli/`）。
   - `pony-agent-core` 与 `src-tauri` 彻底卸载终端 UI 依赖；同步更新根目录 `Cargo.toml`、`package.json`、`scripts/install-cli.mjs`、`scripts/check-cli-release-contract.mjs`、`scripts/check-bin-test-imports.mjs`、clippy 基线与 `.github/workflows/ci.yml`。
2. **第二刀：按依赖 DAG 拆分核心库领域 Crate（`B1795615060457`）**：
   - 在前置拆分完成后，按低耦合有向无环图逐批拆出底座与领域 crate：`pony-protocol`（类型/配置/事件契约）、`pony-storage`（sqlite/session）、`pony-provider`、`pony-tools`、`pony-acp`、`pony-observability`（otel + ponysentry），使 Cargo 的 8-job 预算真正并行调度到多个独立 crate 上。
3. **CI 变更面精细分流（`B1795615060519`）**：
   - 升级 `scripts/ci-scope.mjs` 与 `scripts/ci-scope.spec.sh`，按前端、TUI、Core、Tauri 与文档变更面输出细粒度矩阵开关，保留 fail-safe 兜底全量口径。

### 绑定待办清单（backlog v3 批次 `backlog/log/20261125T135740Z-dev-infra-e843.jsonl`）

| 待办 ID | 阶段 | 优先级 | 类型 | 任务摘要 |
|---|---|---|---|---|
| `B1795589917594` | Phase 1 | P1 | Task | `scripts/cargo-lane.sh` 增加 cgroup v2 探测降级、`ionice` 与 `RUST_TEST_THREADS` 护栏 |
| `B1795615060223` | Phase 1 | P1 | Task | 合并 `crates/pony-agent-core/tests` 266 个集成测试为 4~6 个域测试二进制 |
| `B1795615060280` | Phase 1 | P1 | Task | 接入 `mold` 高速链接器与 `cargo-nextest`，建立 L0–L3 编译测试分层命令与 profile |
| `B1795615060339` | Phase 1 | P2 | Debt | 剥离 `crates/pony-test-support` 消除 `pony-agent-core` 自引用 `test-support` 二次编译 |
| `B1795615060398` | Phase 2 | P2 | Task | 将 TUI（`pony`）与 CLI（`ponycli`）从 `pony-agent-core` 剥离为独立 Workspace Crate |
| `B1795615060457` | Phase 2 | P2 | Task | 按依赖 DAG 拆分 `pony-agent-core` 为底座与领域 Crate |
| `B1795615060519` | Phase 2 | P2 | Task | 升级 `scripts/ci-scope.mjs` 按 Workspace Crate 与前端/TUI/Core 变更面精细分流 CI |

## Alternatives considered

- **A. 拆分为多个独立 Git 仓库（Multi-repo）**：否决。前端（4.6 万行）、`src-tauri`（4,663 行）、TUI 与核心库共享 `contract:check`、`version:check` 四处同步与同一 PR 原子验证契约；拆成多仓会把单次协议修改变成跨仓发版、git rev 锁定与多仓联调负担，且 Rust 编译慢的根因是单 crate 体积过大而非仓库数量。
- **B. 仅提高 `max_jobs_per_lane`（如 12–16）或堆并发解决**：否决。`.agents/notes/implemented/process/2026-11-20-build-engineering-four-fronts.md` 已实测 12 job 相对 8 job 在热编墙钟上零收益（126.27s vs 125.27s），反而增加 1.1 GiB 峰值内存并加剧共享主机卡死风险。
- **C. 跳过 Phase 1 直接全量重写核心库多 Crate**：否决。核心库 21 万行依赖切分跨越数百处内部 `pub(crate)` 边界，直接大爆炸重构周期长且无快速验证手段；先用 Phase 1 几天内收割链接与防卡死红利，可为 Phase 2 每次拆 crate 提供秒级/分钟级安全回归护网。
- **D. 全局关闭 `incremental` 换取 `sccache` 命中率**：否决。既往标定表明 `incremental` 为叶子模块日常修改提供 3.9× 提速（8s vs 31s）且与 `sccache` 硬互斥（`CARGO_INCREMENTAL` 存在时 exit 101），继续保留 `PONY_BUILD_SCCACHE=auto`（仅冷编/`--release` 启用）。

## Acceptance criteria

1. **Phase 1 护栏与分层可验收据**：
   - `scripts/cargo-lane.sh` 支持 cgroup v2 资源限额（带无 user-systemd 安全降级）、`ionice` 与 `RUST_TEST_THREADS` 限制，`bash scripts/cargo-lane.spec.sh` 新增正负样本全绿；人工压测失控单测不会引发整机 Swap 颠簸。
   - `crates/pony-agent-core/tests/` 集成测试二进制数量由 266 个收敛至 ≤ 6 个，全部既有单元测试与集成测试零丢失全绿，`target-test` 占用与全量编译时间给出前后对比收据。
   - L0–L3 分层命令、`.config/nextest.toml` 与 `mold` 通道可用，`bash scripts/build-bench.sh` 给出提速实测对比；`pony-agent-core` 测试期不再因 `test-support` 自引用触发核心库二次编译。
2. **Phase 2 结构拆分可验收据**：
   - `crates/pony-tui` 与 `crates/pony-cli` 独立成 crate 后，仅修改 TUI 文件不触发 `pony-agent-core` 与 `src-tauri` 重编译，`src-tauri` 依赖树不含 `ratatui` 等 TUI 库。
   - 核心库按 DAG 拆出底座与领域 crate 后，无循环依赖，叶子 crate 增量热编墙钟由 ~130s 显著压降至 ≤ 30s；`ci-scope.spec.sh` 正负样本全绿，`npm run verify` 全量门禁通过。

## Risks

1. **`systemd-run --user --scope` 在部分环境不可用或跨平台差异**：如容器/无 D-Bus user session、macOS 与 Windows 均无 Linux cgroup v2。缓解：脚本内明确执行运行时能力探测与安全降级（仅在可用时启用并打印实际生效边界），并在 `cargo-lane.spec.sh` 覆盖有/无 systemd 两条分支。
2. **`RUSTFLAGS` 变更导致既有 target 缓存一次性失效**：直接全局注入 `-C link-arg=-fuse-ld=mold` 会让所有既有构建缓存失效重编。缓解：结合 `build-accel-check.sh` 与独立槽位标定后，再决定在 `.cargo/config.toml` Linux 目标段一次性固化或通过显式开关启用，避免无意切换。
3. **合并集成测试触发 `#[path]` 重复编译、门禁脚本与 CI 参数失配**：266 个集成测试合并过程中可能出现跨测试文件全局静态状态/环境变量碰撞，以及 CI/脚本中写死的 `--test <旧名>` 失效。缓解：按域逐组迁移并逐组验证，同步扫描全仓 `--test` 引用、`check-bin-test-imports.mjs` 基线与 `ci.yml` 配置。
4. **Workspace 拆 crate 触发 `pub(crate)` 边界暴露与 clippy 双基线同步成本**：核心库内部模块互调较多，直接拆分容易暴露内部实现或打破 `scripts/clippy-baseline*.json` 棘轮。缓解：严格按“先 `pony-test-support` 与 `pony-tui`/`pony-cli`，再从低耦合底座 `pony-protocol`/`pony-storage` 逐批推进”的顺序拆分，每拆一刀独立 PR 收口并按平台同步基线。
