# 多会话并发 git 契约：worktree 隔离 + PR 闸门 + 跨平台本地守卫

Status: implemented
Date: 2026-11-18

## 背景

本仓常有多会话同时 AI coding（同一台 dev 服务器上开多个会话/多个 worktree）。实测冲突面不在「谁写错了代码」，而在三类共享状态的争夺：

1. **共享索引**：同一工作区里两个会话同时 `git add` / `git commit`，后者的暂存区覆盖前者。
2. **共享分支尖端**：两个会话推同一远端分支，非快进推送被拒或相互覆盖（force push 直接吃掉对方提交）。
3. **共享文件**：`backlog/tasks.jsonl`、版本四处同步文件（`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` / `.version.json`）在并发提交下必然冲突。

同时暴露两个既有门禁的结构性缺陷（本 ADR 一并收口）：

- **本地守卫在 Linux 上不存在**：`.githooks/pre-commit` 用 `if command -v pwsh` 包住 `scripts/git-guard.ps1`，本机无 pwsh ⇒ 暂存卫生（产物目录/大文件/尾随换行/敏感信息）整段静默跳过。该缺陷已被 `docs/audit/engineering-quality-audit-2026-11-01.md:122` 记录，但一直未修。
- **发版绕过一切闸门**：`npm run release` = `node scripts/release.mjs --push` → `git push origin main --tags`，直推受保护分支语义之外的主干，PR 流程形同虚设。

`gh api repos/lanhui100/pony-agent/branches/main/protection` 返回 `Branch not protected`（HTTP 404），即远端对 `main` 零约束。

## Decision

建立「五层契约 + 两条硬约束」，每层都有一条非零退出的命令做看守：

| 层 | 内容 | 看守命令 |
|---|---|---|
| L1 命名纪律 | 分支 `<类型>/<任务号或主题>`，类型 ∈ {feat, fix, chore, docs, refactor, test, perf, build, ci, revert, release, integrate, backup, exp}；提交信息走 Conventional Commits | `bash scripts/git-guard.spec.sh`（E 段分支名负样本、D 段提交信息负样本） |
| L2 跨平台本地守卫 | `scripts/git-guard.mjs` 单实现覆盖三平台：暂存卫生（产物目录/二进制/20MB/尾随换行/敏感信息）+ pre-push 纪律（禁直推 `main`、禁改写远端既有历史、分支命名）+ commit-msg 规范。`scripts/git-guard.ps1` 退役 | `npm run git:guard:selftest`（11 条正负样本断言） |
| L3 远端闸门 | `main` 受保护：必须走 PR、必须通过必过检查、禁 force push、禁删除；PR 模板带并发安全自查栏 | `gh api repos/lanhui100/pony-agent/branches/main/protection`（非 404） |
| L4 发版路径 | `npm run release` 只做本地 bump + commit + tag（不推送任何东西）；`npm run release:push` 是纯操作：只校验「tag 已存在且指向 `origin/main` 历史」并推送该 tag，任何失败都不留 commit/tag（见 [2026-11-22-release-push-idempotent-and-cli-install-atomic](../bug-fix/2026-11-22-release-push-idempotent-and-cli-install-atomic.md)）。`main` 只能经 PR 进入 | `npm run release:push:selftest`（沙箱 5 组正负样本：缺 tag / 未合入 / 正常推 / 幂等重跑 / 远端指向冲突） |
| L5 文档与技能 | 本 ADR + `AGENTS.md` 常载契约章节 + `.agents/skills/git-flow/SKILL.md` 操作手册 | `npm run adr:check` |

两条硬约束（不随场景放宽）：

1. **`main` 只在人工动作下前进**：PR → 必过检查全绿 → 人点 Merge。禁止脚本直推 `main`；本地 `pre-push` 与远端 ruleset 双重拦截。
2. **一个会话一个 worktree**：`git worktree add -b <分支> /home/dm/pony-agent-worktrees/<主题> main`。主工作区不与其他会话共用索引。
   **2026-11-20 修订**：本条收窄为「并发按需」——单会话串行时直接在主工作区开分支，
   同一时刻 ≥2 个会话写代码时后加入者才建 worktree。见
   [2026-11-20-worktree-on-demand](2026-11-20-worktree-on-demand.md)。

合并策略：普通 PR 用 **Squash**（`main` 线性、一 PR 一提交）；发版 PR（`chore/release-v*`）用 **Merge commit**——squash 会重写提交哈希，使 tag 指向不在 `main` 历史上的提交。

## Alternatives considered

1. **只写纪律文档、不加机械门禁**（成本最低）。否决：本项目已有实证反例——`git-guard.ps1` 的 pwsh 条件分支让整段守卫在 Linux 失效而长期无人发现。「承诺没有非零退出命令」在本仓已被常载命约 2 明确禁止。
2. **每个会话独立 `git clone`**。否决：仓库与 `target/` 产物体积已达 ~80G，多份 clone 会撞磁盘水位；`worktree` 共享 `.git` 对象库，同样获得索引隔离而无需重复存储。
3. **完整 GitFlow（`develop` + `master` 双长期分支 + release 分支）**。否决：单人主导 + 多 AI 会话的节奏下，双长期分支只会制造「两边都要同步」的额外合并面；主干单线 + 短生命周期特性分支已足够，且 `worktree` 才是真正解决并发的手段。
4. **要求第二人 approve（`required_approving_review_count: 1`）**。否决：GitHub 不允许作者 approve 自己的 PR，单人仓库会直接死锁。「人工合并」的必要性由 Merge 动作本身承担，机械门禁负责「必须全绿」；多人协作时再开 CODEOWNERS 即可，无需改本契约。
5. **服务端 `pre-receive` 钩子拦截直推**。否决：GitHub 托管仓库不提供可控的 `pre-receive`；ruleset 是官方等价物且能挂必过检查，本地 `pre-push` 负责快速失败、远端 ruleset 负责最终权威，两层互补。
6. **保留 `--push` 直推 `main` 作为「发版例外」**。否决：一旦留下例外，例外就会成为常规路径（历史上的 `npm run release` 正是如此）。改为「只推 tag + 前置校验已合并」，发版仍是一步命令，但主干前进必须经 PR。

## 后果与代价

- **CI 双跑**：PR 阶段跑一次 `verify`，合并到 `main` 后再跑一次。代价是两倍 CI 时长，收益是「`main` 上的绿」等于「真正被验证过的绿」；接受。
- **`verify` 变长**：新增 `git:guard:selftest`（秒级），可忽略。
- **`git-guard.ps1` 退役**：`bump-version.ps1` 仍在 Windows 侧被 `git-guard.mjs` 调用（`-AutoDetect -Stage`）。无 pwsh 的机器打印一行 NOTE 明确跳过，不再静默。
- **`.meta/gates/backlog-gate.spec.sh` 的 H fixture 同步改指 `git-guard.mjs`**：它原本复制 `git-guard.ps1` 以防「有 pwsh 的机器假红」，现改为无条件依赖 `.mjs`，否则 fixture 自身会红。

## 不覆盖的边界

- **语义冲突仍靠 review**：两个会话改同一函数的不同行为，机械门禁只能发现「同时改了同一行」，发现不了「合起来语义不自洽」。
- **`backlog/tasks.jsonl` 的追加竞争**：`scripts/backlog.mjs` 的任务级租约只保护同一 ID，不保护账本文件本身；并发 `record` 仍可能丢行，靠「record 后立即提交 + 冲突人工合并」兜底（`backlog/README.md` 已声明）。
- **`--dry-run` 不能当作远端拦截的验收证据**：`git push --dry-run` 只做客户端引用检查，不触发 GitHub 服务端 ruleset 评估（实测 `git push --dry-run --no-verify origin HEAD:main` 返回 0，看起来「推上去了」）。远端是否真生效只认 `gh api repos/<owner>/<repo>/rules/branches/main`——它返回 `pull_request` / `required_status_checks` / `non_fast_forward` / `deletion` 四条 active 规则。
- **`core-platform-check` 暂未纳入 required checks**：matrix job 的 check 上下文带 runner 后缀（`core-platform-check (ubuntu-latest)` 等），且两侧是长跑平台矩阵。起步只 require `verify` + `governance`，避免 PR 因平台矩阵排队而长时间无法合并；待实测 CI 时长可接受再逐个追加。
- **版本 auto-bump 在并发下仍是冲突源**：`pre-commit` 的 `-AutoDetect -Stage` 会让每个会话都 bump 版本，四处同步文件在合并时冲突。本 ADR 不改其行为（属既有约定），但契约文档要求「并发会话下把版本 bump 收敛到发版 PR 一次完成」。
