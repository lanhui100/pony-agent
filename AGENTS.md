# AGENTS.md —— Pony Agent 工程治理总则与常载规则

> 本文是面向所有 Agent、开发团队与协作工具的统一导航索引与常载行动准则。

## 1. 体系入口地图（Tier 索引）

为杜绝事实重复与知识腐化，本项目所有事实严格执行分层存放：

- **文档标准与写作契约**：[`docs/AGENTS.md`](docs/AGENTS.md)（tier 分类法、写作规则、防腐化自查表）
- **工程宪章与治理根**：[`.meta/`](.meta/)
  - 成熟度与治理状态：[`.meta/meta.yaml`](.meta/meta.yaml)（当前定级：L2 承诺可验+文档分层）
  - 核心架构宪法：[`.meta/constitution/constitution.md`](.meta/constitution/constitution.md)
  - 门禁索引说明：[`.meta/gates/README.md`](.meta/gates/README.md)
  - Rust lint 表与口径：[`.meta/lints/README.md`](.meta/lints/README.md)（规则真值源 + 应用模式裁剪理由 + 实测底数；门为 `npm run rust:lint`）
  - 文档分层清单：[`.meta/docs-tier/README.md`](.meta/docs-tier/README.md)
- **技术决策记录 (ADR)**：[`.agents/notes/`](.agents/notes/)（最高优先级决策真源，技术方案选型、现在时记录、替代方案考量；受 `adr-verify` 强门禁保护）
- **架构与设计指南**：[`docs/architecture/`](docs/architecture/)（系统拓扑、模块交互、运行时设计）
- **待办任务与执行账本 (Backlog)**：[`backlog/`](backlog/)（唯一活账本 [`tasks.jsonl`](backlog/tasks.jsonl)，支持任务级并发租约锁与行级强校验；契约见 [`backlog/README.md`](backlog/README.md)，由 `scripts/backlog.mjs` 驱动、`backlog-gate`（lint + doctor）门禁约束）
- **工程复用技能库**：[`.agents/skills/`](.agents/skills/)
- **历史归档档案（已冻结退役，只读）**：
  - OpenSpec 规范归档：[`openspec/`](openspec/)（历史规格与设计草案，见 [`RETIRED.md`](openspec/RETIRED.md)）
  - 历史决策归档：[`.agents/notes/archived/`](.agents/notes/archived/)（早期 0001~0123 篇 ADR 均已全量重构迁移并封印，核心架构见 `.agents/notes/implemented/architecture/`）

---

## 2. 核心架构约束与不变量

1. **Rust 核心与前端渲染职责分离**：
   - 调度、沙箱执行、持久化与大模型交互必须位于 `crates/pony-agent-core` 及 `src-tauri`。
   - 前端（Vue 3 / TypeScript）仅作交互界面与状态展示，模型网络流量严禁直接走前端 `fetch`。
2. **外部安全边界（Fail-Closed）**：
   - 桌面打开外部链接统一调用 `src/lib/open-external-url.ts`，Rust 后端强制执行 HTTPS 协议与严格 host 白名单（真值源：`src-tauri/src/platform.rs` 的 `ALLOWED_OPEN_HOSTS`，当前四项：`github.com`, `api.github.com`, `exa.ai`, `s3.local.ponyjob.top`），Windows 平台使用 `ShellExecuteW` 唤起默认浏览器，严禁使用命令拼接。白名单增删必须与本条文档同提交更新。
3. **版本四处同步契约**：
   - `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 必须保持严格同步。
   - 任何涉及版本的提交由 `scripts/check-version-sync.ps1` 校验。

---

## 3. 本地与 CI 门禁命令速查

- **全量合规自检**：`npm run verify`（依次校验版本同步、决策记录、账本门、前端单测、构建、Rust 检查）
- **CI 变更面分流**：`.github/workflows/ci.yml` 的 `changes` job 用 `scripts/ci-scope.mjs` 判定——纯文档/账本 PR（`docs/`、`backlog/`、`openspec/`、`.agents/notes/`、`.agents/skills/`、根级 `*.md`）跳过 Rust 全量步骤，其余路径照旧全量跑。口径 fail-safe 到「跑全量」（`.meta/` 下的 markdown 也算代码面）。负样本自检 `bash scripts/ci-scope.spec.sh`，真源 ADR `2026-11-20-ci-scope-split`。
- **决策记录校验**：`npm run adr:check`（判定真源 `.agents/skills/write-adr/verify-note.sh`，pre-commit 调 `adr-verify.sh`）
- **版本同步校验**：`npm run version:check`
- **前端测试**：`npm test`
- **Rust 编译检查**：`npm run cargo:check:shared`
- **Rust lint 棘轮（clippy 表在册）**：`npm run rust:lint`——存量在 `scripts/clippy-baseline.json` **按平台分册**冻结，新增/恶化非零退出；改表/改 `clippy.toml` 阈值必须重基线（`npm run rust:lint:update`，检出上涨时须显式 `--allow-growth`）；门自检 `npm run rust:lint:selftest`。平台分册缺失时门 exit 2（在该平台跑一次 update 即可，勿套用别平台口径）。口径与波次见 [`.meta/lints/README.md`](.meta/lints/README.md)
- **并发构建预算（强制）**：并行 lane 的构建/测试一律走 `bash scripts/cargo-lane.sh <cargo 参数>`（4 job + 跨 target 目录全局 flock + nice 5）；release / 全量回归只由 Lead 走 `bash scripts/cargo-lane.sh --heavy …` 独跑且与 lane 编译互斥；`uptime` 负载持续 > 1.5×核数即停一条 lane。契约真源 `.dev-team/build-budget.json`（`python3 ~/.agents/skills/dev-team/scripts/dev-build-budget-lint.py .` 校验），负样本自检 `bash scripts/cargo-lane.spec.sh`。背景见 `.dev-team/rfc-2026-11-17-build-budget.md`（4 条 target 目录 × 16 job ⇒ load 45.86，release 90min 未完成 vs 单跑 3m07s）。
- **构建产物保留深度（分层）**：`bash scripts/artifacts-status.sh` 看体积与磁盘水位（>70% 非零退出，`--json` 供 CI）；清理只用 `bash scripts/clean-targets.sh`——**默认 light 只删可再生产物**（`target-check*`/`target-test*`/`target-perf`/`target-codex-*`），根 `target/`（含 `target/release`）是热缓存**不删**；`--with-frontend` 才删 `dist`/`coverage`/`playwright-report`/`test-results`；`--deep` 才动根 `target/`，仅限四类触发：工具链升级、C/link 失败（`failed to run custom build`）、依赖大换血、磁盘 <20%。产物一律不入库；证据类落 `docs/release/evidence/`，过程类落 `/tmp` 并收口清理。负样本自检 `bash scripts/artifacts-tools.spec.sh`。当前实测产物 ≈80G（target 46G + target-test 23G + target-check 9.4G），故禁止「多槽位同时长驻」。
- **可观测性归属先决**：新增/修改对外接口、网络 IO、后台任务、LLM 调用或 UI 交互前，先更新 `docs/observability.yaml` 的四维归属（适用 / N-A + 结构性理由）与收口点；验收跑
  `~/.agents/skills/observability-onboarding/scripts/audit-diff.sh` 与 `check-declaration.sh`（判定口径见该 skill 的 `references/declaration-rubric.md`）。N-A 必须写结构性理由，不得为让门禁变绿而伪造 `/metrics` 或改 `applicable`。
## 4. Git 并发与合并契约（多会话强制）

多会话同时 AI coding 时，冲突源头不是「谁写错代码」，而是共享索引、共享分支尖端与共享文件。以下四组硬规则无场景例外（决策真源：[ADR 2026-11-18](.agents/notes/implemented/process/2026-11-18-multi-session-git-contract.md)，操作手册：[`git-flow` skill](.agents/skills/git-flow/SKILL.md)）：

1. **写会话隔离（并发才建 worktree）**：单会话串行时直接在主工作区开 `<类型>/<主题>` 分支；**同一时刻 ≥2 个会话要写代码**时，后加入者必须 `git worktree add -b <类型>/<主题> ~/pony-agent-worktrees/<主题> origin/main`，主工作区不与其他会话共用索引，收口后 `git worktree remove <路径>`。判定口径与回退条件见 ADR [2026-11-20-worktree-on-demand](.agents/notes/implemented/process/2026-11-20-worktree-on-demand.md)。
2. **命名纪律**：分支 `<类型>/<任务号或主题>`，类型 ∈ {feat, fix, chore, docs, refactor, test, perf, build, ci, revert, release, integrate, backup, exp}；提交信息走 Conventional Commits（`fix(tui): 修复焦点丢失`）。
3. **`main` 只能经 PR 前进**：推特性分支 → 开 PR → CI `verify` 全绿 → **人工点 Merge**。本地 `pre-push`（`scripts/git-guard.mjs`）与远端 ruleset 双重拦截直推与历史改写；灾备确需直推时显式 `PONY_GUARD_ALLOW_MAIN_PUSH=1` 并在提交信息说明理由。
4. **发版不改写主干**：`npm run release`（本地 bump + commit + tag）→ 走 PR 合并 → `npm run release:push`（只推 tag 触发发布）。普通 PR 用 **Squash**，发版 PR（`chore/release-v*`）用 **Merge commit**，否则 tag 会指向不在 `main` 历史上的提交。

配套门禁：`npm run git:guard:selftest`（本地守卫正负样本自检，已接入 `npm run verify` 与 CI governance）、`gh api repos/lanhui100/pony-agent/branches/main/protection`（远端保护在册，非 404）。共享文件（`backlog/tasks.jsonl`、四处同步版本文件）合并前须确认无并发写入；并发场景下版本 bump 收敛到发版 PR 一次完成。

<!-- BEGIN constitution -->

## 常载命约（Standing Orders）

1. 任何非平凡变更都要落为 `.agents/notes/` 下一条带 `## Alternatives considered` 的决策记录（程序与校准样例见 `.agents/skills/write-adr/SKILL.md`）。
2. 凡机械可查的承诺，配一条非零退出的命令；机器到不了的，显式标注「靠 review」。
3. 删除代码前先搜消费者；恢复条件立法，拒绝「以后可能用得上」式怀旧。
4. 每个事实一个家：文档治理规则的家是 `docs/AGENTS.md`（agent 自动加载），人读契约是 README；改行为/契约的那次提交同步更新对应文档。
5. 非平凡交付必须在 backlog 账本留痕（record / start / resolve）；账本一致性与租约对账由 `backlog-gate`（lint + doctor）机械校验，证据成立性靠 review。
<!-- END constitution -->
