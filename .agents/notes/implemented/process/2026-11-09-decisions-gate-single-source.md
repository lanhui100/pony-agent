# Agent Note: 决策记录门禁单一判据与 0038 撞号消解

Status: implemented

配套决策：[ADR 0109](../../../docs/decisions/0109-decisions-gate-single-source-and-collision-rule.md)
（本轮同时立法「撞号消解规则」，改写 `docs/decisions/README.md` 的编号纪律条目）。

## Decision

1. 判定真源唯一：`scripts/check-decisions.mjs`（R1–R7）；`scripts/verify-decisions.ps1` 自建判据全部删除，
   只剩 `& node check-decisions.mjs @args` 转发 + 无 node 时 fail-closed。
2. `.githooks/pre-commit` 决策门改为 node 优先 / pwsh 次之 / 两者皆无 ⇒ `exit 1`（拒绝提交而非静默放行）。
   暂存卫生与版本 bump（`git-guard.ps1`）仍只在 pwsh 侧生效——这是已知不对称，非本轮目标。
3. `npm run decisions:check` 插入 `npm run verify` 链（版本同步之后、单测之前）；`decisions:fix`
   只在显式调用时回填索引。
4. 撞号消解：`0038` 保留给引用面更大的 TUI 设计系统篇，`0038-wave35-suite-remediation.md` →
   `0110-wave35-suite-remediation.md`（`git mv` 保留历史，正文一字未改，顶部加「编号修订」块）；
   存活指针改 2 处（`docs/architecture/plan-execution-approval.md`、README 索引行），
   封存件（`99_LOGS`、`.dev-team/contracts/wave-36-pa138-145.md`、`state-archive-through-wave41.json`）不回改。
5. `.agents/skills/write-adr/SKILL.md` 撤掉两条 ad-hoc `grep -L` 检查与「目录一致性靠人工核对」的
   过期说法，改指向同一门禁；取号步骤补「先验 R6 未占用」。
6. `AGENTS.md` §3 门禁速查补 `decisions:check` 一行（工具链变更与文档同提交）。

## Alternatives considered

- **装 pwsh 让 Linux 也跑 ps1 版**：落选——治理门依赖解释器安装，等价于「不装就没有门」；27 条历史违规即其后果。
- **ps1 / mjs 双实现并存**：落选——mjs 已多出 R4–R7，两份判据必然漂移，且违规会在两侧给出不同结论。
- **只进 CI 不进 pre-commit**：落选——违规 ADR 先落进 `main` 再红，回滚成本高于本地 50ms。
- **改 TUI 设计系统那篇的号**：落选——引用面 8+（0042/0043/0076/0078/0079、`docs/INDEX.md`、wave-39 文档）
  vs wave-35 的 2 处存活引用，让少数方让路。
- **留 tombstone 文件**：落选——再次触发 R6，违反「同一编号只允许对应一个文件」。
- **把 `0101`/`0105`/`0106`/`0107`/`0108` 的索引行一起提交**：落选——那些 ADR 文件尚未落地，
  HEAD 里会出现 R5 悬挂链接；改为「ADR 与其索引行同提交」，本轮 staged README 显式排除这 5 行。
- **`--fix-index` 一把梭**：落选——会给他人未跟踪的 ADR 机械回填摘要行，越界改别人的落点。

## 收据

- 违规基线（对 HEAD 树跑同一判据）：103 篇 / 27 条 = R2×2 + R4×24 + R6×1。
- 本轮后：`node scripts/check-decisions.mjs` → `通过：109 篇`，exit 0，实测 50ms。
- 正/反用例：`npx vitest run tests/decisions-gate.spec.ts` → 14 passed（含 R1–R7 变异、fixIndex 幂等、
  CLI 退出码 0/1/2、真实仓状态/目录/索引三项闭合）。
- 提交自洽性：commit 后用 `git worktree add --detach` 起纯净 HEAD 树跑门禁 ⇒ 必须 exit 0（验 R4/R5 无悬挂）。
