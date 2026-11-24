# Agent Note: Git Flow 本地守卫补漏与 Squash Worktree 安全清理

Status: implemented

Date: 2026-11-24

关联与部分接替：[2026-11-18-multi-session-git-contract](2026-11-18-multi-session-git-contract.md)。

## Problem

对当前仓 Git Flow 与本地钩子实测审查暴露出五处具体错落与漏口：

1. **首次推送新分支时密钥扫描静默漏扫**：`.githooks/pre-push` 调 `node scripts/check-secrets.mjs --pre-push` 时不传 range；新分支尚未设置 `@{upstream}`，`check-secrets.mjs` 回退到 `targetRange = "HEAD"` 并执行 `git diff -U0 HEAD`，实际只扫描未提交的工作区改动，完全放过本次分支已提交但尚未推送的所有 commits。
2. **Windows `pre-commit` auto-bump 与并发契约冲突**：`scripts/git-guard.mjs` 的 `maybeAutoBump` 在存在 `pwsh` 时会自动运行 `bump-version.ps1 -AutoDetect -Stage`，在 Linux 上却静默跳过；既制造双平台分裂，也直接违背「并发会话把版本 bump 收敛到发版 PR 一次完成」的既有契约，且在并发 PR 中制造四处版本文件冲突。
3. **`pre-commit` 行尾检查读工作区而非暂存区**：`git-guard.mjs` 用 `fs.readFileSync(absPath)` 校验尾随换行，先 `git add` 再修改工作区（或只在工作区修复未 re-stage）会造成误报或放行脏 blob；`BLOCKED_PATH_PATTERNS` 亦漏掉 `target-perf`、`.venv`、`.scratch`、`backlog/.locks`、`backlog/.state`。
4. **Squash merge 导致 worktree/分支长期堆积**：主干采用 Squash and merge，`git branch --merged origin/main` 对已合入的特性分支完全失明，实测累积 30 个已结束 worktree 无人清理；同时 `pre-push` 忽略所有 `(delete)` 推送，未在本地快速拦截删 `main` 或删远端 tag。
5. **多处文档与 `verify` 口径漂移**：`npm run verify` 的 `secrets:check` 只跑 `--staged`（commit 后跑等于空转），与 CI 的 `--all` 不一致；`AGENTS.md` 仍引用 404 的 `branches/main/protection` 端点；`git-flow/SKILL.md` 与 `PULL_REQUEST_TEMPLATE.md` 仍引用已退役的 `backlog/tasks.jsonl`；`package.json` 缺少 `prepare` 自动挂接 `.githooks`。
6. **v3 多 worktree 租约对账误拦与可观测门软链崩溃**：`origin/main` 存在其他已合并批次（如 `B277..B280`）的 `in_progress` 任务，新建 worktree 执行 `backlog.mjs start` 建立本地 `backlog/.locks/` 后，`leaseConsistency` 把所有跨批次 `in_progress` 任务也当成本工作区缺锁判红，导致本地 `pre-commit` 被他人的历史任务卡死；此外 `node_modules/` 尾斜杠在 `.gitignore` 下不忽略 worktree 根 `node_modules` 软链，`scripts/observability/gate.mjs` 的 `ruleA` 仅判 `fs.existsSync` 随即对软链执行 `readFileSync` 触发 `EISDIR` 崩溃。

## Decision

1. **修复 `scripts/check-secrets.mjs --pre-push` 范围解析**：新增并导出 `resolvePrePushRange(cwd)`，无 `@{upstream}` 时依次回退 `merge-base(HEAD, origin/main|main|origin/master|master)..HEAD`，无主干引用则回退空树 `4b825dc642cb6eb9a060e54bf8d69288fbee4904..HEAD`；全部 git 调用改用 `execFileSync` 数组传参并支持 `cwd` 注入；在 `tests/check-secrets.spec.ts` 新增真实临时仓回归用例。
2. **加固 `scripts/git-guard.mjs`（并移除 `maybeAutoBump`）**：
   - 删除 `maybeAutoBump`（及无用 `which`），版本 bump 唯一合法入口收敛到 `npm run release`（发版 PR）及手工 `npm run version:*`；恢复条件：若未来切回单主干串行提交且放弃发版 PR 集中 bump，才重新评估恢复提交期自动 bump。
   - `guardPreCommit` 改用 `git ls-files -s` + `git cat-file blob :<path>` 校验暂存区真实 blob 大小与尾随换行（跳过符号链接与 gitlink）；`BLOCKED_PATH_PATTERNS` 补全 `target-perf`、`target-core-test`、`src-tauri/target-perf`、`.scratch`、`venv`/`.venv`、`node_modules`、`backlog/.locks`、`backlog/.state`。
   - `guardPrePush` 不再跳过 `(delete)`：本地快速拦截删除 `main`（除非设置 `PONY_GUARD_ALLOW_MAIN_PUSH=1`）与删除 `refs/tags/*`，继续放行普通特性分支删除。
3. **新增 `scripts/git-prune-worktrees.mjs`（+ `git:prune:selftest`）**：结合 `gh pr list --state merged|open`（或 `--offline` 祖先判定）、`git status --short`（忽略根目录 `node_modules` 软链）与 Linux `/proc/<pid>/cwd` 进程占用检测，默认 dry-run，`--apply` 才删除已合并且干净无占用的历史 worktree 与分支。
4. **对齐 `package.json` 与文档真源**：`package.json` 增加 `prepare` 幂等配置 `core.hooksPath=.githooks`，`verify` 改调 `secrets:check:all` 并纳入 `git:prune:selftest`；同步更新 `AGENTS.md`（Ruleset 端点 `rules/branches/main`、Node 单真源版本校验、`git:prune`）、`.agents/skills/git-flow/SKILL.md`、`.github/PULL_REQUEST_TEMPLATE.md`（v3 `backlog/log/*.jsonl`）、`scripts/release.mjs`（`origin/main`）与 `.meta/gates/README.md`。
5. **修复 v3 多 worktree 租约对账边界与可观测门软链兼容**：`scripts/backlog.mjs` 的 `applyEvent` 记录 `row._start_batch = batch.batchId`，`leaseConsistency` 在本地 `.locks/` 启用且存在 `.state/batch` 时，仅对属于当前工作区活动批次的 `in_progress` 任务强制要求本地 `.lock`，跨批次历史 `in_progress` 降级为 `note` 提示；`scripts/observability/gate.mjs` 在 `ruleA` 读取前校验 `fs.statSync(abs, { throwIfNoEntry: false })?.isFile()`，跳过目录及目录软链，并在 `tests/acceptance/wave47/gate-conformance.spec.ts` 增加双引擎一致的 `C5` 回归用例。

## Alternatives considered

1. **在 `pre-push` 中仅用 `origin/main..HEAD` 硬编码**：否决。已跟踪上游且连续多轮 push 的分支只需扫 `@{upstream}..HEAD`，且部分离线/初始化仓库主分支可能叫 `main`/`master`；分级回退更稳妥。
2. **保留 `maybeAutoBump` 并为 Linux 重写一份 Node 版**：否决。问题根因不是缺 Linux 实现，而是每次代码提交都改共享版本文件会与并发 PR 制造必发冲突，违反「发版 PR 集中 bump」的既有决策。
3. **直接用 `git branch --merged` 清理 worktree**：否决。GitHub Squash merge 后原分支提交哈希不在 `main` 历史中，`--merged` 完全失效，必须结合 PR 状态 + 脏区/进程占用的 fail-closed 保护。

## Consequences

- 新分支首次推送携带未推敏感信息会在本地 `pre-push` 被立即拦截，不再依靠远端 CI 事后兜底。
- Windows 与 Linux 在 `pre-commit` 阶段行为完全一致，并发特性分支不再产生无谓版本文件冲突。
- 本地 `npm run verify` 与 CI 均运行全库 `secrets:check:all`，消除了「commit 后本地 verify 假绿」的不一致。
