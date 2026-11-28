# Agent Note: CI 变更面分流收窄、Tauri 宿主测试补位与扩展点载体路径纠偏

Status: implemented

## Problem

10-crate 拆分（[2026-11-25-build-test-layering-and-workspace-crate-split.md](2026-11-25-build-test-layering-and-workspace-crate-split.md)）把 `governed_executor.rs` 与 MCP 配置解析迁到了 `crates/pony-tools`，但三处机械承诺没有跟着迁移，形成三类"看起来在守、实际没守"的假绿：

1. **扩展点归属表误报载体未落地**：`scripts/gen-extension-points.mjs` 仍探测拆分前路径 `crates/pony-agent-core/src/agent/governed_executor.rs`。文件搬走后 `exists()` 恒 false，生成物把已落地的内建工具执行器写成"载体未落地"；而 `--check` 只比对生成物与仓库内文件、不校验事实源是否新鲜，所以 `npm run ext:check` 一直 exit 0。
2. **CI 的"Tauri 宿主回归测试"名不副实**：`.github/workflows/ci.yml` 里该 step 实际重跑 `-p pony-agent-core --test session_storage_suite --test runtime_suite --test tools_stop_suite`，与上一步 Core 全包 nextest 完全重复（零增量信号）；而 `src-tauri` 自身的 lib 单测（`platform` / `goal_events` / `goal_resume` / `tauri_adapter` 四个 `#[cfg(test)]` 模块）与 `src-tauri/tests/` 下 5 个集成回归在 CI 上**零执行点**。
3. **纯前端改动付满 Rust 门**：`scripts/ci-scope.mjs` 把 `src/**` 一律判 `needs_rust=true`，一个 `.vue` 文案改动要跑 Windows 近 30min 的 cargo 全量 + 三平台矩阵；与此同时 `docs/contracts/**`（Rust 侧 `conformance` / `ts_contract_snapshot` 测试的读盘事实源）被 `docs/` 白名单当纯文档放行，方向恰好相反。

## Decision

1. **扩展点生成器跟随 crate 拆分迁路径**：`gen-extension-points.mjs` 的载体探测根由 `crates/pony-agent-core/src/agent` 改为 `crates/pony-tools/src`（`governed_executor.rs` 与 `mcpServers` 关键词同源），并重生成 `docs/architecture/extension-points.md`。脚本头注释同时改为真实事实源路径，并记下"`--check` 不校验事实源新鲜度 ⇒ 移动载体必须同步本脚本（靠 review）"。
2. **CI 用包名跑真宿主测试**：删除重复的 core suite 重跑，改为 `nextest run -p pony-agent`（lib 单测 + 5 个集成回归），门控由 `needs_core` 换成语义正确的 `needs_tauri`；`core-platform-check` 的 Linux 分支补 `cargo test -p pony-agent --lib`，让宿主 lib 单测至少有一个非 Windows 执行点。
3. **`ci-scope` 按编译面而不是目录名分流**：
   - `src/**`（`src/types/` 除外）判 `frontend-only`：`needs_frontend=true`、`needs_rust=false`；
   - `src/types/**` 仍判 `needs_rust=true`（Rust serde 线缆键的 TS 镜像，漏改即前端静默丢字段）；
   - `docs/contracts/**` 从文档白名单移出，判 `needs_rust=true` + `needs_core=true`（Rust 测试读盘事实源）；
   - 新增 `scope_reason=frontend-only`，`scripts/ci-scope.spec.sh` 补 I1–I7 七条正负样本（旧用例 I 断言"前端源码触发 Rust"随之改为分流断言）。

## Alternatives considered

- **方案 A（采纳）：路径与门控随拆分同步迁移，`ci-scope` 显式区分编译面**。三条漂移同源（拆分后路径/包名/编译面变化），一次收口；`frontend-only` 用显式前缀表而不是"看起来像前端"的推断，fail-safe 方向仍是"拿不准就跑全量"。
- **方案 B：只修扩展点路径，CI 与 ci-scope 维持现状**。落选：CI 的重复 suite 与零执行点是可机械判定的覆盖缺口，纯 UI 改动的 Rust 门则是可量化的机器浪费；留着等于把"门禁存在"写成"门禁有效"。
- **方案 C：把 `src/**` 全判 `frontend-only`（含 `src/types/`）**。落选：`src/types/` 是 Rust 线缆的 TS 镜像，`contract:check` 只查"Rust 有、TS 无"单向，改 TS 而漏改 Rust 侧会被放过；保留 `needs_rust=true` 是刻意的非对称 fail-safe。
- **方案 D：把 `tests/**` 也判 `frontend-only`**。落选：`tests/` 同时承载 vitest 与 playwright 之外的历史 Rust 副本与契约夹具，当前无机械证据表明它可以整体脱离 Rust 面，保持保守。

## Consequences

- `npm run ext:check` 重新能抓"载体落地状态"的真实漂移（同一提交内生成物与事实源一致）。
- CI 首次覆盖桌面宿主：`-p pony-agent` 的 lib 单测与 5 个集成回归进入 `verify`（Windows，`needs_tauri`）与 `core-platform-check`（Linux lib 单测）。
- 纯前端 PR 的 Rust 门省掉；`docs/contracts` 改动不再被文档白名单放过。
- 剩余同类漂移（`[until]` 方括号豁免、clippy/version 漏新 crate、vitest 覆盖率脱钩、`check-graph-run-lock-scope` 未接线）仍留在
  [proposed/process/2026-11-28-post-split-gates-and-test-realignment.md](../../proposed/process/2026-11-28-post-split-gates-and-test-realignment.md)，
  本记录只落其中已获批执行的 CI/口径部分。
- 机器门禁：`bash scripts/ci-scope.spec.sh`（I1–I7 新增样本）与 `npm run ext:check`；`gen-extension-points` 的事实源新鲜度仍靠 review（`--check` 结构上判不了）。
