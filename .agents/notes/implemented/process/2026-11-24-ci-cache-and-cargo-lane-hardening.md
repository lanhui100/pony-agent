# Agent Note: CI 缓存分槽、Cargo 入口收敛与 bin-as-module 棘轮收口

Status: implemented

Date: 2026-11-24

关联与补足：[2026-11-20-build-engineering-four-fronts](2026-11-20-build-engineering-four-fronts.md)、[2026-11-20-ci-scope-split](2026-11-20-ci-scope-split.md)、[2026-11-24-size-gate-ci-wiring](2026-11-24-size-gate-ci-wiring.md)。

## Problem

对 CI 工作流、Cargo 构建测试入口与遗留待办 `B1795484554592` 的实测审查发现四类具体错落：

1. **CI 缓存双跑且 `target-test` 长期冷编**：`.github/workflows/ci.yml`、`release.yml` 与 `lint-baseline-bootstrap.yml` 中 `actions-rust-lang/setup-rust-toolchain@v1` 默认自带 `rust-cache`，同时后续又再次调用 `Swatinem/rust-cache@v2`，造成同一 job 重复恢复/保存缓存；更严重的是 `verify`（`windows-latest`）与 `core-platform-check`（`ubuntu-latest`）均只缓存 `target-check`，实际测试写入的 `target-test` 每次冷编译（实测 Windows `cargo:test:shared` 耗时 1181s–1274s，占总时长 62%）。此外 `ci.yml` 缺少 PR `concurrency` 自动取消陈旧 run，`ubuntu-latest` 运行两次 `apt-get update && apt-get install`。
2. **CI 与本地 `verify` / 发版一致性缺口**：`verify` job 未执行 `npm run build`（前端构建）、`contract:check`、`contract:cli`、`ext:check` 与 `src-tauri` 回归测试（`cargo:test:regression` 亦漏掉 `stop_registry_wave60`）；`release.yml` 的 `build-cli` 未 pin `toolchain: "1.91.1"`、未传 `--locked` 与 `RUSTFLAGS: "-D warnings"`，也未注入 `PONY_RELEASE_BUILD: "1"` 导致官方 CLI 产物默认上报环境为 `development`。
3. **`package.json` 与 CI 仍存在绕过 `cargo-lane.sh` 的裸调用**：`cli:build`、`cargo:check`、`cargo:test`、`cargo:test:lib`、`cargo:test:regression`、`conformance:test` 与 `ci.yml` 的 `core-platform-check` 测试步骤直接调用 `cargo`，绕过并发预算与全局构建锁；另外本地工作区曾草拟的 `cargo-lane.sh` 180s 测试超时存在把 `flock` 等锁时间计入测试预算、子串匹配任何位置 `"test"` 参数且 180s 对全量测试过窄的问题。
4. **`B1795484554592`（bin-as-module 门禁）未收口**：仅扫描顶层 `crates/*/tests/*.rs` 漏掉子目录，只有文件级白名单无单文件次数限制，`--update-baseline` 缺 `--allow-growth` 护栏，且未接入 `npm run verify` 与 CI。同时 `pony-agent-core/tests/` 累积 263 个顶层测试文件（含 49 份 `#[path = "../src/bin/tui.rs"]` 与 1 份 `cli.rs`），每份二进制重复内嵌全套 bin 单元测。

## Decision

1. **修复工作流缓存、并发与门禁覆盖**：
   - `.github/workflows/ci.yml`、`.github/workflows/release.yml`、`.github/workflows/lint-baseline-bootstrap.yml` 统一在 `setup-rust-toolchain@v1` 显式配置 `cache: false`，由 `Swatinem/rust-cache@v2` 承担单一真源；为 `target-check` 与 `target-test` 配置互不冲突的 `prefix-key`（`v0-rust-check` / `v0-rust-test`）与 `workspaces`。
   - `.github/workflows/ci.yml` 增加 PR 级 `concurrency`（仅 PR `cancel-in-progress: true`，不取消 `main` push）；合并 `ubuntu-latest` 两次 `apt-get` 为单次 `--no-install-recommends` 安装；`verify` 改跑 `npm run build`（生成 `dist/`）、契约全项（`contract:check` / `contract:cli` / `ext:check` / `contract:bin-imports*`）与 `npm run cargo:test:regression -- --no-fail-fast`。
   - `.github/workflows/release.yml` 的 `build-and-release` 与 `build-cli` 统一 pin `toolchain: "1.91.1"`；`build-cli` 补齐 `PONY_RELEASE_BUILD: "1"`、`CARGO_INCREMENTAL: "0"`、`RUSTFLAGS: "-D warnings"` 与 `cargo build --release --locked --bin pony`。
   - `.github/workflows/ponygo-governance.yml` 挂接 `setup-node@v4`（Node 22）及零 `node_modules` 依赖的静态契约与构建治理自检，在 Ubuntu 侧数秒级拦截脚本与基线回归。
2. **Cargo 入口收敛到 `invoke-rust-target.mjs` + `cargo-lane.sh`**：
   - `scripts/invoke-rust-target.mjs` 与 `scripts/invoke-rust-target.ps1` 新增 `release -> target` 槽位，`buildLaneInvocation` 在 `release` 槽位或 `--release` 参数下自动前插 `--heavy`；`package.json` 的 `cli:build`、`cargo:check`、`cargo:test`、`cargo:test:lib`、`cargo:test:regression`（补入 `--test stop_registry_wave60`）、`conformance:test` 及 `ci.yml` 非 Windows 测试步骤全部收敛经此入口调用；同步更新 `tests/invoke-rust-target-slots.spec.ts`。
   - `scripts/cargo-lane.sh` 新增 `detect_cargo_subcommand` 精确识别 Cargo 主子命令（忽略 `-- test` 等过滤词），仅当主子命令为 `test` 时在 `flock`/mkdir 锁**内部**注入 `timeout --signal=TERM --kill-after=15s "$PONY_TEST_TIMEOUT"`（默认 `600s`，`PONY_TEST_TIMEOUT=0|none` 关闭）；`scripts/cargo-lane.spec.sh` 增加 CI 无本地 skill 时的 P5 字段校验回退并补齐 P17–P20 正负样本。
3. **收口 `B1795484554592`（`bin-as-module` 棘轮门禁）**：
   - `scripts/check-bin-test-imports.mjs` 递归扫描 `crates/*/tests/**/*.rs` 与 `src-tauri/tests/**/*.rs`，剥离 `//`/`/* ... */` 注释后匹配跨行/单行 `#[path = "...src/bin/..."]`，按文件记录允许次数（v2 基线 `scripts/bin-as-module-baseline.json`，移除 `updated_at` 噪声）；`--update-baseline` 在无基线或出现文件/次数上涨时 fail-closed，必须显式 `--allow-growth` 才放行；接入 `package.json`（`contract:bin-imports` / `verify`）与 `ponygo-governance.yml`；`scripts/check-bin-test-imports.spec.mjs` 全部运行在 `--root <mkdtemp>` 沙箱内（不污染真实测试树），覆盖 7 类正负样本。
   - **恢复与后续拆分路径**：当前仅用棘轮锁死存量（50 文件 / 50 处）零增长；待 `tui.rs` / `cli.rs` 纯函数与状态机逐步迁入 `crates/pony-agent-core/src/` 后，再按 `contract:bin-imports:update` 收窄白名单直至清零，同时将 `crates/pony-agent-core/tests/` 按域合并以降低冷链接成本。

## Alternatives considered

1. **保留 `setup-rust-toolchain` 内置缓存并删除 `Swatinem/rust-cache@v2`**：否决。`setup-rust-toolchain` 默认只按 `. -> target` 策略管理单目录且缺少按 `target-check` / `target-test` 双 `prefix-key` 显式分槽的能力，无法覆盖 `SLOT_TARGET_DIRS` 约定。
2. **仅把 `timeout` 放在 `flock` 外层并将阈值设为 180s**：否决。并行 lane 等锁时间与全量 test crate 链接时间会被误算入测试超时；放到锁内并将默认值定为 600s 可同时避免等锁误杀与死循环拖死宿主。
3. **本 PR 直接重构全部 50 个 `bin-as-module` 集成测试与 263 个 test 文件**：否决。一次性抽离 `14,251` 行 `tui.rs` 公共符号风险与变更面过大；先通过次数级棘轮门禁止血并独立推进模块抽取，符合渐进式交付约束。

## Consequences

- `verify` 与 `core-platform-check` 的 `target-test` 首次合入预热后可复用依赖与测试增量产物，消除同一 job 双份 Rust 缓存上传/恢复及 Ubuntu 重复 `apt-get`。
- `npm run` 与 CI 中剩余 Cargo 直调入口全部受 `cargo-lane.sh` 并发上限与全局锁保护，`stop_registry_wave60` 与前端 `vite build` 正式进入 CI `verify` 必过链路。
- 官方 `build-cli` 产出的 `pony` 二进制与桌面端一致携带 `PONY_RELEASE_BUILD=1`、`--locked` 与 `-D warnings` 检查；任何在测试代码中新增 `#[path = "...src/bin/..."]` 的提交会在本地与 CI 秒级阻断。
