# Agent Note: worktree 隔离从「逐会话强制」收敛为「并发按需」

Status: implemented

## Problem

2026-11-18 的多会话 git 契约把「一个会话一个 worktree」写成无条件硬约束
（ADR [2026-11-18-multi-session-git-contract](2026-11-18-multi-session-git-contract.md)
硬约束 2）。实践下来这条规则的收益边界很清楚：

- 它解决的是**索引争用**——两个会话同时 `git add` / `commit`，后者的暂存区覆盖前者。
  只有真并发时这个争用才存在。
- 单人串行工作时它提供零收益，却把「改代码」变成三步：建 worktree → 干活 → 收口删除。
- 更关键的是它已被事实绕过：主工作区长期直接工作在特性分支上，`git worktree list` 只有一条
  （= 主工作区本身）。规则的作者与会话自己都没在执行它。

规则被静默绕过比规则更宽松更糟：它让「契约是否真的被执行」不可查，也让后来者分不清
哪条是硬约束、哪条是仪式。

## Decision

worktree 隔离改为**并发按需**（部分取代原 ADR 硬约束 2）：

- **单会话串行工作** ⇒ 直接在主工作区开特性分支，不建 worktree。收口仍是「合 PR + 删分支」。
- **同一时刻 ≥2 个会话要写代码** ⇒ 后加入的会话必须
  `git worktree add -b <类型>/<主题> ~/pony-agent-worktrees/<主题> origin/main`，
  主工作区不与他人共用索引。
- 判定责任在使用者：开第二个并发写会话前先 `git worktree list` 看清现状。

其余三条硬约束**不变**：分支命名 `<类型>/<任务号或主题>` + Conventional Commits；
`main` 只经 PR 前进且合并动作必须人工点；发版只推 tag（发版 PR 用 Merge commit 合并）。

## Alternatives considered

1. **保持无条件强制**。否决：规则已被事实违反，继续写在契约里只会稀释「硬约束」这个词；
   要么执行要么改，中间态最差。
2. **彻底删掉 worktree 规则**。否决：索引争用是真实故障（后提交覆盖前者暂存区），
   并发时没有等价替代机制；删掉等于把已知故障面重新打开。
3. **用「单工作区串行化 + `git stash`」代替 worktree**。否决：多 AI 会话天然并发，
   串行化等于人为排队；且 stash 栈是全局共享状态，冲突模式更难诊断。
4. **每个会话独立 `git clone`**。否决（沿用原 ADR 结论）：仓库 + `target/` 产物规模已达 ~80G，
   多份 clone 会撞磁盘水位；worktree 共享对象库即可获得索引隔离。

## Reintroduction conditions

出现任一情形，worktree 恢复为逐会话无条件强制（即回到原 ADR 硬约束 2 的原文）：

- 使用模式改变：出现第二个真人协作者，或仓库改为多人提交；
- 实际发生一次「主工作区索引被并发会话覆盖」的事故（表现为提交里混入不属于本次会话的文件、
  或他人暂存内容被提交）；
- CI 或工具链开始依赖「主工作区永远在 main 分支」这一假设。

## Consequences

- 单人日常少两步（建/删 worktree）：分支生命周期变成「切分支 → 干活 → PR → 删分支」。
- 契约重新可查：`git worktree list` 只有主工作区 = 正常单会话态；出现 ≥2 条 = 有人在并发写，
  该会话应自带 worktree。
- 文档同步：`AGENTS.md` 第 4 节、`.agents/skills/git-flow/SKILL.md` 与原 ADR 的硬约束 2
  已链到本条。
