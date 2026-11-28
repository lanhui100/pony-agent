# Agent Note: 假闭环 ADR 清理、仓库跟踪卫生收口与文档防漂移策略

Status: proposed

> **人工已批准（2026-11-28）**：方案照此执行，阻塞解除；落地进度见 `backlog:B1795831359205 / B1795831359338`。
> 本文件**仍留在 `proposed/`**直到代码在同一提交里落地——批准 ≠ 实现，提前迁 `implemented/` 即假闭环。
>
> **落地进度（仓库卫生部分，B1795831359338）**：删除根目录 12 个一次性报告 JSON、`AGENT.md`（与 `AGENTS.md` 正面冲突）、`playground/rust-hello/{hello.exe,hello.pdb}`、`scripts/file-size-baseline.json.bak`；4 篇散落调研 md 迁入 `docs/archive/root-reports/`（附归档说明）；`docs/INDEX.md` 与 `ci.yml` 注释改指 `AGENTS.md`。新增根目录白名单门 `scripts/check-root-allowlist.mjs` + 5 项正负样本自检（接入 `verify` 与 `ponygo-governance.yml`）：根级新增被跟踪文件必须显式登记，否则 exit 1。
> **独立验证收据（2026-11-28，非本批代码作者执行）**：`npm run verify` exit 0；`npm run test:unit` 184 files / 1661 passed / 13 skipped；`bash .meta/gates/ci-verify.sh` 通过（四段）；`bash .meta/gates/no-residue.sh` 0（16 条）；`node scripts/check-root-allowlist.mjs` 0；`node scripts/check-placeholder-tests.mjs` 0；GitHub CI 在 push 事件上 `verify` 与 `ponygo-governance` 均为 SUCCESS。
> **未落地**：`docs/observability.yaml` 17 次重复注记与 `reqwest::Client` 补录、9 个新 crate + `src-tauri` 的 README 覆盖（判据 1.9c）、`docs/` 中 78+238 处退役路径引用、`.dev-team/` 46MB 分期瘦身、B074/B079 二选一裁决 —— 均需独立波次或人工拍板。

## Problem

全仓审计发现了三类严重的治理账本真实性漂移与仓库卫生膨胀问题：

1. **假闭环 ADR 与恒真占位测试（账本与现实严重脱节）**：
   - [2026-11-12-kebab-case-decision.md](../../implemented/architecture/2026-11-12-kebab-case-decision.md)（对应工单 B079）标记为 `Status: implemented`，且 B079 已在 `backlog/archive/tasks-2026.jsonl` 标为 `done`，但 `crates/pony-tools/src/workspace_hotloader.rs` 中零相关实现，唯一关联测试 `crates/pony-agent-core/tests/runtime_suite/wave2_b079_kebab_case.rs:4` 仅有一行 `assert!(true);`。
   - [2026-11-12-watchdog-design.md](../architecture/2026-11-12-watchdog-design.md)（对应工单 B074）在全仓无 `notify` 依赖与任何 200ms 监听代码，唯一测试 `crates/pony-agent-core/tests/tools_stop_suite/wave2_b074_watchdog.rs:4` 同样为 `assert!(true);`，但 B074 已被标为 `done`。
2. **`proposed/` 树滞留与重复覆盖**：
   - [2026-11-11-skill-invocation-catalog-dsh-alignment.md](../architecture/2026-11-11-skill-invocation-catalog-dsh-alignment.md) 第 1/2/3(配置)/布尔修复已落地，仅剩 watchdog/kebab-case 未解而长期滞留 `proposed/`；
   - [2026-11-16-process-group-reap-gap-remediation.md](../architecture/2026-11-16-process-group-reap-gap-remediation.md)（B205–B208 全 `done`）已被 `implemented/bug-fix/2026-11-16-residual-orphan-leak-sweep.md` 取代（仅 `process.rs:26` 一行注释遗漏）却未迁移；
   - [2026-11-17-turn-incremental-persist.md](../architecture/2026-11-17-turn-incremental-persist.md)（B209 `done`）已通过替代接口 `turn_flow.rs:1595` `flush_step_boundary_events` 落地却未迁出 `proposed/`。
3. **根目录双根冲突、跟踪垃圾与 `observability.yaml` 膨胀**：
   - 根目录 `AGENT.md`（声称事实写 `AGENT.md`、使用 OpenSpec、`docs/decisions/`、`management/task-system/`）与 `AGENTS.md` 正面冲突；
   - 根目录被 Git 跟踪 12 个一次性报告 JSON（`purity_*.json`、`test_report.json` 等）、4 篇未归类调研/报告、`playground/rust-hello/{hello.exe,hello.pdb}`（1.3MB Windows 二进制与 PDB）、`scripts/file-size-baseline.json.bak`；`codex-openai` 同时存在于 `.gitmodules` 与 `.gitignore:6`；`.dev-team/` 跟踪 **1,691 文件 / 46MB**；
   - `docs/observability.yaml` 膨胀至 2,119 行（顶部「收口补录」注释重复粘贴 17 次），且 `reqwest::Client` 清单漏记 `updater.rs` 与 `tui.rs`；
   - `docs/` 中 **78 个文件**引已删 `docs/decisions`（参见 [2026-11-11-retire-docs-decisions-to-notes.md](../../implemented/simplification/2026-11-11-retire-docs-decisions-to-notes.md)）、**238 个文件**引已删 `management/task-system`，9 个新拆 crate 与 `src-tauri/` 缺 `README.md`（违反判据 1.9c）。

## Proposal

建议人类审阅拍板后，按以下四项策略完成治理收口与机械防回归建设：

1. **假闭环 ADR 裁决与恒真测试封杀**：
   - 请人类二选一裁决 B074（技能文件 watchdog）与 B079（kebab-case 技能名校验）：若确需实现，则按 `write-adr` 把原 ADR 降级或重开新 `backlog` 任务补齐真实实现与测试（移除 `assert!(true)`）；若决定不做，则将 `2026-11-12-kebab-case-decision.md` 与 `2026-11-12-watchdog-design.md` 经 `git mv` 迁入 `.agents/notes/rejected/architecture/`（附明确拒绝理由）并删除两个占位测试文件。
   - 对 3 篇已落地 `proposed/` ADR 执行状态迁移/归档（拆分 `skill-invocation-catalog-dsh-alignment` 已落地部分为 `implemented/`，归档 `process-group-reap-gap-remediation` 与 `turn-incremental-persist` 并执行 `archive-verify.sh seal`）。
2. **清理根目录垃圾、退役 `AGENT.md` 与收敛双入口脚本**：
   - 删除根目录 `AGENT.md`、12 个临时 JSON、`playground/rust-hello/{hello.exe,hello.pdb}`、`scripts/file-size-baseline.json.bak`、`scripts/trace-audit-loop.py`（未接线模拟桩），把 4 篇根目录散落报告按 `docs/AGENTS.md` 归位或归档，解除 `codex-openai` submodule 与 `.gitignore` 冲突；
   - 下线旧决策双入口 `scripts/check-decisions.mjs` / `scripts/verify-decisions.ps1`（CI/release 统一改调 `adr-verify`/`verify-note.sh` + `archive-verify.sh`）；
   - 清理 `docs/observability.yaml` 17 次重复粘贴注释并补齐 `updater.rs`、`tui.rs` 出网客户端记录，同时补齐 9 个新 crate + `src-tauri/` 的 `README.md`（或显式入 `.meta/docs-tier/exemptions`）与修复 `README.md`、`runtime.md`、`overview.md`、`constitution.md`（同步为 v3）漂移。
3. **`.dev-team/` 分期瘦身**：保留近两波活跃证据与基线脚本，其余历史波次执行记录（1,600+ 文件 / 46MB）评估迁出仓库（Release asset / 外部归档）或压缩清理，严禁改写 git 历史。
4. **新增四条机械门禁防止回流**：
   - 禁 `assert!(true)` 占位测试扫描（接入 `verify`）；
   - 文档禁区扫描（拦截宿主机绝对路径 `C:/Users/`、失效相对路径与新增 `docs/decisions/` / `management/task-system/` 引用）；
   - 根目录 tracked 文件白名单与判据 1.9c（`crates/`/`src-tauri` README 覆盖）接入 `ponygo-governance.yml`；
   - `.meta/constitution/constitution.md` `<!-- sync-body -->` 与 `AGENTS.md` 字节一致性校验。

## Alternatives considered

- **方案 A：真实化假闭环状态 + 清理跟踪垃圾 + 增设机械白名单门禁（推荐）**。既解决当前账本/ADR 信任危机，又防止后续 Agent 继续提交根目录临时文件、占位测试或滞留提案。
- **方案 B：只删根目录 12 个 JSON 与二进制，不动假闭环 ADR 与 `.dev-team/`**。落选：允许 `implemented/` 存在零代码的虚构决策记录，会破坏整个 ADR 体系作为单一架构真源的可信度。
- **方案 C：使用 `git filter-repo` 改写主干历史彻底抹除 `.dev-team/` 体积**。落选：违反仓库「`main` 历史不可改写」的并发 Git 契约，会破坏所有本地克隆与 worktree。

## Acceptance criteria

1. 人工明确裁决 B074 与 B079 的去留（补实作 vs 迁 `rejected/`），全仓测试目录 `assert!(true)` 占位用例清零。
2. `.agents/notes/proposed/` 中已落地的 3 篇历史提案全部完成迁移/拆分并通过 `npm run adr:check`。
3. 根目录除合法配置文件与 `README.md` / `AGENTS.md` / `CLAUDE.md` 外无游离 `.json` / `.md` / `.exe` / `.pdb`；9 个新 crate 与 `src-tauri` 完成文档覆盖（判据 1.9c 绿灯）。
4. `docs/observability.yaml` 消除 17 次重复注记并与真实 `reqwest::Client` 分布一致。

## Risks

- 大批量清理 `docs/` 中 78+238 处旧路径引用时，历史归档文件夹（`docs/archive/`、`docs/imports-archived/`、`docs/audit/raw/`）属历史快照不应篡改，门禁扫描需明确排除历史归档只读区。
- `.dev-team/` 内部脚本（如 `check-lock-discipline.py`、`build-budget.json`、`artifacts-budget.json`）被门禁与 `AGENTS.md` 活跃引用，瘦身前必须先排查所有脚本和门禁引用清单。
