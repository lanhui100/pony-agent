---
name: git-flow
description: 何时用：多会话/多 worktree 并行开发时的 git 操作——开分支、提交、推送、提 PR、合并、发版，或遇到「不能推 main」「钩子拦截了提交」「worktree 怎么收口」这类问题时。单人 + 多 AI 会话场景下的主干单线契约，含直推拦截与发版路径。
---

# git-flow —— 多会话并发 git 操作手册

契约真源：[ADR 2026-11-18-multi-session-git-contract](../../notes/implemented/process/2026-11-18-multi-session-git-contract.md)。
常载摘要见根 `AGENTS.md` 第 4 节。冲突时以 ADR + `scripts/git-guard.mjs` 代码为准。

## 四条不可协商的规则

1. 一个会话一个 worktree，主工作区不与他人共用索引。
2. 分支名 `<类型>/<任务号或主题>`；提交信息 Conventional Commits。
3. `main` 只能经 PR 前进，合并动作必须人工点。
4. 发版只额外推 tag，不推进 `main` 历史。

## 日常流程

```bash
# 1) 开隔离工作区（从最新 main）
git -C ~/pony-agent fetch origin main
git -C ~/pony-agent worktree add -b feat/B999-my-change ~/pony-agent-worktrees/B999-my-change origin/main
cd ~/pony-agent-worktrees/B999-my-change

# 2) 干活 + 提交（钩子自动跑：敏感信息 / 产物卫生 / 尾随换行 / 提交信息规范）
git add <paths>
git commit -m "feat(tui): 一句话说清改了什么"

# 3) 推送 + 开 PR（pre-push 会拒绝无关分支名与历史改写）
git push -u origin feat/B999-my-change
gh pr create --fill

# 4) CI verify 全绿后，人工在 GitHub 点 Squash and merge

# 5) 收口
git -C ~/pony-agent worktree remove ~/pony-agent-worktrees/B999-my-change
git -C ~/pony-agent branch -d feat/B999-my-change
```

## 被钩子拦下时怎么判断

| 拦截信息 | 含义 | 正确处理 |
|---|---|---|
| 禁止提交构建/运行产物目录 | 暂存区混入 `target*/`、`dist/`、`coverage/`、`sessions/` | `git restore --staged <path>`，检查 `.gitignore` |
| 文件必须以恰好一个换行符结尾 | 文本文件尾部 0 个或 ≥2 个换行 | 修文件，别用 `--no-verify` |
| 禁止提交超过 20MB 的文件 | 大资源入库 | 单独评审其版本管理策略（不要硬塞） |
| 提交信息不符合 Conventional Commits | 首行不是 `<type>(<scope>): <subject>` | 改信息；merge/fixup 前缀已豁免 |
| 禁止直接推送 main | 想推主干 | 推特性分支走 PR；灾备才用 `PONY_GUARD_ALLOW_MAIN_PUSH=1` |
| 拒绝改写远端已有历史 | 本地分支不是远端快进的延续 | 先 `git fetch` 再 rebase；确认该分支是否属于他人 |
| 分支名不符合约定 | 缺类型前缀或用了裸名 | `git branch -m feat/B999-topic` |

预检命令（不用真的提交就能验证守卫没退化）：`npm run git:guard:selftest`。

## 发版流程（main 受保护后）

```bash
git switch -c chore/release-v0.1.200 origin/main
npm run release                      # 本地 bump + commit + 打 tag，不推送
git push -u origin chore/release-v0.1.200
gh pr create --fill                  # → 人工用 Merge commit 合并（不要 squash）
npm run release:push                 # 只推 tag；未合并进 origin/main 会被拒绝
```

**发版 PR 必须用 Merge commit**：squash 会重写提交哈希，tag 将指向不在 `main` 历史上的提交，追溯断链。

## 并发下的共享文件

- `backlog/tasks.jsonl`：用 `node scripts/backlog.mjs record/start/resolve`，不要手改；任务级租约只保护同一 ID，账本追加仍靠「record 后立即提交」兜底。
- 版本四处同步（`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` / `.version.json`）：`pre-commit` 的 auto-bump 只在 Windows（有 pwsh）生效；并发会话下把版本 bump 收敛到发版 PR 一次完成，避免每个会话各 bump 一次。

## 不覆盖的边界

- 语义冲突（两人改同一函数的不同行为）机械门禁看不见，靠 review 与 PR 模板自查栏。
- 远端 ruleset 是最终权威；本地钩子只做快速失败，`--no-verify` 绕过后仍会被远端拒绝。
- 强制改写他人分支、删远端分支不在自动拦截范围，属人工纪律。
