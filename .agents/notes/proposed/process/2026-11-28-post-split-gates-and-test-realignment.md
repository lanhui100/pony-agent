# Agent Note: 架构拆分后的门禁再对齐与前端测试去递归方案

Status: proposed

## Problem

在完成 [2026-11-25-build-test-layering-and-workspace-crate-split.md](../../implemented/architecture/2026-11-25-build-test-layering-and-workspace-crate-split.md)（10-crate 拆分）及前端 `src/stores/runtime.ts` 拆解为 12 个子模块后，多条核心机械门禁未随结构演化同步更新，导致出现系统性门禁失明与 60 倍测试性能退化：

1. **前端单测嵌套递归拖慢 60 倍**：`tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts:57,248` 在 `vitest run` 内部再起子进程重跑全量 Vitest（`--no-file-parallelism`），实测导致 `npm run test:unit` 总耗时 **466.74s** 中该单文件占 **459.06s（98.3%）**，日常反馈回路被严重阻塞。
2. **文件规模豁免语法漏洞**：`scripts/check-file-size.mjs:89` 帮助信息示例写了 `[until YYYY-MM-DD]`，但 `:433` 正则 `^until\s+(\d{4}-\d{2}-\d{2})\b` 不匹配方括号，导致 `crates/pony-tui/src/tui.rs:4`（14,705 行）与 `crates/pony-tools/src/workspace_hotloader.rs:1`（3,920 行）写 `[until 2026-12-31]` 后被判为“永不过期的永久豁免”。
3. **Clippy 棘轮丢失拆分 crate（参考 [2026-11-18-rust-quality-gate-and-skills.md](../../implemented/process/2026-11-18-rust-quality-gate-and-skills.md)）**：`scripts/clippy-baseline.json` 仅扫 `--package pony-agent-core`，拆分前占 ~798 条告警的 `pony-tui` 与 `pony-cli` 现已彻底脱离 clippy 看守；同时 `clippy-baseline-src-tauri.json`（3,323 条）重复计入了 core 7 crate 的 3,083 条。
4. **Vitest 覆盖率门禁脱钩**：`vitest.config.ts:33-40` 仅统计 `App.vue`、`HomeWorkspace.vue`、`HomeSessionSidebar.vue` 与 `src/stores/runtime.ts`（拆分后仅剩 78 行装配壳），真正的 **7,012 行**核心状态机（`src/stores/runtime/*.ts` 12 文件）完全不在覆盖率统计中。
5. **版本同步、扩展点与 CI 盲区**：
   - `scripts/check-version-sync.mjs:8-12` 与 `scripts/release.mjs:12-16` 仅覆盖 3 个 crate，漏掉新拆出的 6 个底层 crate；
   - `scripts/gen-extension-points.mjs:123` 仍查旧路径 `crates/pony-agent-core/src/agent/governed_executor.rs`，致 `npm run ext:check` 假绿且 `docs/architecture/extension-points.md:11` 误报内建工具「载体未落地」；
   - `.github/workflows/ci.yml` 全文零执行 `src-tauri`（`-p pony-agent`）单测、重复执行 3 个 core suite（`:168` 与 `:177`）、Linux/macOS 仅 check `pony-agent-core`；
   - `scripts/check-graph-run-lock-scope.mjs` 沦为未接线死脚本（且已有 1 条 stale 豁免 `mod.rs:2040`）；
   - `scripts/ci-scope.mjs:108` 对任意前端改动无条件设 `needs_rust=true`，触发三平台 Rust 编译浪费。

## Proposal

执行一次「门禁与测试对齐」专项修复，把所有因架构拆分而脱钩的机械承诺重新接回 CI 与本地验证链：

1. **剥离 Vitest 递归重跑**：将 `v10-baseline-freeze.spec.ts` 中的 V10-3 全量递归子进程重跑移出默认 `npm run test:unit`（改由独立脚本 `test:v10-freeze` 或在 CI 中直接串联执行，避免测试套娃），保留 V10-1/2/4 轻量契约测试，使 `npm run test:unit` 耗时从 466s 压回 ~8–15s。
2. **修复 `file-size-exempt` 解析并对齐尺度（参考 [2026-11-12-file-size-gate-module-split-not-baseline-bump.md](../../implemented/process/2026-11-12-file-size-gate-module-split-not-baseline-bump.md)）**：在 `scripts/check-file-size.mjs` 显式拒绝 `[until ...]` 语法并统一帮助文本；修复 `tui.rs:4` 与 `workspace_hotloader.rs:1` 的括号写法，使其真实受 `until 2026-12-31` 到期约束。
3. **补齐多 Crate Clippy 与版本覆盖**：将 `pony-tui`、`pony-cli`、`pony-test-support` 纳入 `scripts/check-clippy-baseline.mjs` 扫描面（独立基线或按 workspace package 列表分册，消除 `src-tauri` 对 core 7 crate 的 3,083 条重复计数）；将 `check-version-sync.mjs` 与 `release.mjs` 扩至全部 10 个 core workspace crate。
4. **重新锚定前端覆盖率、扩展点与锁门禁**：
   - 将 `vitest.config.ts` 的 `coverage.include` 扩展到 `src/stores/runtime/**/*.ts`（排除 `__tests__` 与 fixtures）并重校准阈值；
   - 修正 `scripts/gen-extension-points.mjs:123` 为 `crates/pony-tools/src/governed_executor.rs` 并重生成 `extension-points.md`；
   - 将 `node scripts/check-graph-run-lock-scope.mjs --self-test && node scripts/check-graph-run-lock-scope.mjs` 接入 `ponygo-governance.yml` 与 `npm run verify`，清理过时豁免行；
   - 在 `ci.yml` 增加 `cargo test -p pony-agent --lib`、消除重复的 3 个 core suite、收窄 `ci-scope.mjs` 对纯 UI 改动不触发全量 Rust（除非触及 `src/types/` 或契约文件）。

## Alternatives considered

- **方案 A：逐项恢复拆分后的机械门禁对齐 + 剥离嵌套测试（推荐）**。恢复 `verify` 与 CI 的真实信号强度，同时把单次前端测试成本降低 97%，显著提升多 Agent 局部验证速度。
- **方案 B：维持现状，仅靠人工 Review 盯 `tui.rs` / `runtime/*.ts` 与 6 个底层 crate 版本**。落选：违背常载命约第 2 条（凡机械可查的承诺必配非零退出命令），已有实证表明一旦脱管立即出现版本分裂与 14,705 行超级巨石永久豁免。
- **方案 C：直接删除 `v10-baseline-freeze.spec.ts` 全部用例并永久放宽 `file-size` 豁免**。落选：V10-1/2/4 仍有冻结基线守护价值，不应连带移除；放宽规模门等于放弃规模治理。

## Acceptance criteria

1. `npm run test:unit` 在 20s 内完成且不含测试进程递归嵌套重跑；独立 V10 冻结校验命令可按需执行并 exit 0。
2. `scripts/check-file-size.mjs` 对 `[until ...]` 语法测试样例返回非零退出码，`tui.rs` 与 `workspace_hotloader.rs` 过期日期被真实解析。
3. `npm run rust:lint` 覆盖 `pony-tui` 与 `pony-cli`；`npm run version:check` 覆盖全部 10 个内部 core crate；`npm run ext:check` 验证的 `extension-points.md` 正确展示 `governed_executor.rs` 已落地。
4. `vitest.config.ts` `coverage.include` 覆盖 `src/stores/runtime/**/*.ts` 且 `npm run test:ui-guard` 通过。
5. `ci.yml` 包含 `pony-agent`（`src-tauri`）单测且无重复 suite 执行；`ponygo-governance.yml` 运行 `check-graph-run-lock-scope.mjs`。

## Risks

- 把 `pony-tui`/`pony-cli` 重新纳入 clippy 棘轮及扩展 `vitest` 覆盖率时，需要一次性同步生成新 crate 的基线与校准覆盖率阈值，若校准不当可能短暂阻塞合入。
- 收窄 `ci-scope.mjs` 对前端改动触发 `needs_rust` 的条件时，必须通过 `ci-scope.spec.sh` 正负样本验证触及 `src/types/`、`docs/contracts/` 或 `src-tauri/` 仍强制触发 `needs_rust=true`。
