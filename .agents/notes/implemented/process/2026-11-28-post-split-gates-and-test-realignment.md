# Agent Note: 架构拆分后的门禁再对齐与前端测试去递归

Status: implemented

## Problem

在完成 [2026-11-25-build-test-layering-and-workspace-crate-split.md](../../implemented/architecture/2026-11-25-build-test-layering-and-workspace-crate-split.md)（10-crate 拆分）及前端 `src/stores/runtime.ts` 拆解为 12 个子模块后，多条核心机械门禁未随结构演化同步更新，导致出现系统性门禁失明与数十倍测试性能退化：

1. **前端单测嵌套递归**：`tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts:57,248` 在 `vitest run` 内部再起子进程重跑全量 Vitest（`--no-file-parallelism`）。本机实测 `npm run test:unit` 总耗时 **349.56s**，其中该单文件 **244.20s**、V10-3 单例 **231.56s（占该文件 94.8%）**；日常反馈回路被严重阻塞。
2. **文件规模豁免语法漏洞**：`scripts/check-file-size.mjs:89` 帮助信息示例写了 `[until YYYY-MM-DD]`，但 `:433` 正则 `^until\s+(\d{4}-\d{2}-\d{2})\b` 不匹配方括号，导致 `crates/pony-tui/src/tui.rs:4`（14,740 行）与 `crates/pony-tools/src/workspace_hotloader.rs:1`（3,920 行）写 `[until 2026-12-31]` 后被判为「永不过期的永久豁免」。
3. **Clippy 棘轮丢失拆分 crate（参考 [2026-11-18-rust-quality-gate-and-skills.md](../../implemented/process/2026-11-18-rust-quality-gate-and-skills.md)）**：`scripts/clippy-baseline.json` 仅扫 `--package pony-agent-core`（实测覆盖 7 个 crate），拆分出去的 `pony-tui`（linux 实测 3,743 条）与 `pony-cli`（3,056 条）彻底脱离 clippy 看守；同时 `clippy-baseline-src-tauri.json` 重复计入了 core 7 crate 的告警。
4. **Vitest 覆盖率门禁脱钩**：`vitest.config.ts:33-40` 仅统计 `App.vue`、`HomeWorkspace.vue`、`HomeSessionSidebar.vue` 与 `src/stores/runtime.ts`（拆分后仅剩 78 行装配壳），真正的 **7,176 行**核心状态机（`src/stores/runtime/**` 12 文件）完全不在覆盖率统计中。
5. **版本同步、扩展点与 CI 盲区**：
   - `scripts/check-version-sync.mjs:8-12` 与 `scripts/release.mjs:12-16` 仅覆盖 3 个 crate，漏掉新拆出的 6 个底层 crate；
   - `scripts/gen-extension-points.mjs:123` 仍查旧路径 `crates/pony-agent-core/src/agent/governed_executor.rs`，致 `npm run ext:check` 假绿且 `docs/architecture/extension-points.md:11` 误报内建工具「载体未落地」；
   - `scripts/check-graph-run-lock-scope.mjs` 沦为未接线死脚本（且已有 1 条 stale 豁免 `mod.rs:2040`）。

## Decision

一次「门禁与测试对齐」专项修复，把因架构拆分而脱钩的机械承诺重新接回 CI 与本地验证链。分两条独立 PR 落地（P0-1 测试性能、P0-2 门禁对齐），以下为最终形态：

1. **V10-3 降级为显式命令**：`v10-baseline-freeze.spec.ts` 的 V10-3 改为 `PDS_V10_FULL_SUITE=1` 显式 opt-in，默认 skip 并打印可见原因；新增 `scripts/v10-freeze.mjs` / `npm run test:v10-freeze` 注入开关（不走 `VAR=1 cmd` 前缀——Windows cmd.exe 不认），CI 在 `needs_frontend` 时跑一次。判据强度不变，仍是真实子进程 exit 0 + 用例计数自检。实测 `test:unit` 349.56s → **93.27s**；`test:v10-freeze` 4/4 通过（V10-3 仍真实全量重跑 245s）。
2. **显式拒绝方括号豁免语法**：`check-file-size.mjs` 对 `[until ...]` 返回 `bracket-until` 违规并统一帮助文本；`tui.rs` / `workspace_hotloader.rs` 改为无方括号写法，真实受 `until 2026-12-31` 约束。新增 `scripts/check-file-size.spec.mjs`（6 条正负样本，接 `size:check:selftest` + `npm run verify` + `ponygo-governance.yml`），钉住「语法写错 ≠ 永久豁免」。
3. **补齐多 Crate Clippy 与版本覆盖**：新建 `scripts/clippy-baseline-tui.json`（1,310 键 / 3,743 条）与 `clippy-baseline-cli.json`（1,132 键 / 3,056 条），对应 `npm run rust:lint:tui` / `rust:lint:cli`，接 `verify`、`ponygo-governance.yml` 与 CI 的 Linux 分支（新基线目前仅 `platforms.linux` 分册，缺分册按既有约定 exit 2，故暂只挂 Linux，win32/darwin 分册由 `lint-baseline-bootstrap` 生成后放宽 `if`）。`check-version-sync.mjs` 与 `release.mjs` 扩至全部 9 个内部 core crate（`pony-test-support` 是 `publish = false` 的测试专用 crate，版本独立，不纳入同步）。
4. **重新锚定前端覆盖率与锁门禁**：
   - `vitest.config.ts` 的 `coverage.include` 扩到 `src/stores/runtime/**/*.ts`（排除 `__tests__`、`fixtures`、`types.ts`），阈值按新口径重锚为 87/79/87/88（实测 87.54/79.90/87.37/88.11）；`verify` 改用 `test:coverage`，`test:ui-guard` 保留 3 个 UI spec 的子集口径（独立 include + 80/71/85/79 阈值）。
   - `gen-extension-points.mjs` 的 `SRC` 修正为 `crates/pony-tools/src`，`extension-points.md` 重生成后「内建工具」载体状态转正为已存在。
   - `check-graph-run-lock-scope.mjs` 接入 `npm run verify` 与 `ponygo-governance.yml`（`lock-scope:selftest` + `lock-scope:check`）；删除已失效的 `mod.rs:2040` stale 豁免，并把自检改用**合成豁免表**，使它不再依赖真实豁免表非空。

## Alternatives considered

- **逐项恢复拆分后的机械门禁对齐 + 显式化嵌套测试（采纳）**。恢复 `verify` 与 CI 的真实信号强度，同时把单次前端测试成本降低约 73%，显著提升多 Agent 局部验证速度。
- **维持现状，仅靠人工 Review 盯 `tui.rs` / `runtime/**` 与 6 个底层 crate 版本**。落选：违背常载命约第 2 条（凡机械可查的承诺必配非零退出命令），已有实证表明一旦脱管立即出现版本分裂与 14,740 行巨石永久豁免。
- **删除 `v10-baseline-freeze.spec.ts` 全部用例并永久放宽 `file-size` 豁免**。落选：V10-1/2/4 仍有冻结基线守护价值，不应连带移除；放宽规模门等于放弃规模治理。
- **保留 V10-3 在默认 `test:unit` 内、仅调大超时**。落选：它验证的信号与外层 run 完全重合（同代码、同用例、同断言），调超时只把「慢」变成「更慢地慢」。

## Consequences

- `npm run test:unit` 从 349.56s 降到 93.27s（≈-73%），但**代价是本地单跑不再覆盖「全量里再跑一遍全量」**这个双重确认。补偿点：CI 的 `needs_frontend` 步骤执行 `test:v10-freeze`，发版前亦可手动跑。若未来要恢复默认路径，置 `PDS_V10_FULL_SUITE=1` 即可。
- 新增两份 clippy 基线目前只有 `linux` 分册，CI 步骤因此**只挂 Linux**（已知缺口而非已覆盖）。补齐 win32/darwin 需跑 `lint-baseline-bootstrap` 工作流并 review 合入，届时把该步骤 `if` 放宽为 `runner.os != 'Windows'`。
- 覆盖率门从「只统计 78 行装配壳」变成「统计 7,176 行主体 + 三个 UI 文件」，阈值随之从 82/73/82/82 重锚到 87/79/87/88。这意味着**新代码进入 `src/stores/runtime/**` 时覆盖率压力显著上升**——这是设计意图，不是副作用。
- `verify` 变长：新增 `size:check:selftest`、`lock-scope:selftest`、`lock-scope:check`、`rust:lint:tui`、`rust:lint:cli`，且 `test:unit` 换成 `test:coverage` + `test:ui-guard`。总时长仍低于修复前（旧 `test:unit` 单独就 349s）。
- `pony-test-support` 明确**不纳入**版本同步（`publish = false`，版本 0.1.0 独立）。若将来它被发布，须同步重新评估本决定。
- 未落地项（原提案含、本次刻意不做）：CI 中 `src-tauri`（`-p pony-agent`）单测缺失、`:168`/`:177` 重复跑 3 个 core suite、`ci-scope.mjs:108` 对任意前端改动无条件 `needs_rust=true`。这三条属 CI 编排优化，与门禁失明正交，已登记为 P1 `B1795831357829`，不在本决策范围内。
