# Agent Note: 决策记录门禁单一判据与撞号消解规则

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0109-decisions-gate-single-source-and-collision-rule.md` 之归档快照，仅供溯源。

## 背景

`docs/decisions/` 的四条纪律（`Status:` 行、目录即状态、索引完整、编号唯一）此前只由
`scripts/verify-decisions.ps1` 把守，而 `.githooks/pre-commit` 的调用条件是 `command -v pwsh`：
Linux 开发机上这道门**整段不执行**。缺口因此静静累积——把判据（`scripts/check-decisions.mjs`）
对着 HEAD 的树跑一次，103 篇 ADR 报出 **27 条违规**：

- `R2`×2：`0099-session-storage-event-sourced-refactor.md` 写 `Status: accepted`（状态枚举里根本没有这个词）、
  `0104-provider-failover-and-upstream-policy.md` 写 `proposed` 却留在主目录；
- `R4`×24：`0025`~`0104` 等 24 篇从未登记进 README 索引；
- `R6`×1：**同一编号 `0038` 对应两个文件**——`0038-tui-design-system-monochrome-restraint.md` 与
  `0038-wave35-suite-remediation.md`。两篇都是 implemented、都在主目录、都真实被引用，撞号来自两次
  互不可见的并发写入（README 只写了「编号一经分配永不复用、永不修改」，没写撞号时谁让路）。

同一天 `0107`（proposed 留在主目录且未进索引）也是同一缺口的产物。结论：问题不是某篇 ADR 写错，
而是**判据不可执行**——机械可查的承诺没有非零退出的命令，就等于没有承诺。

## 决策

1. **一份判据，一个可执行入口**：`scripts/check-decisions.mjs` 是判定真源，R1–R7 全在其中
   （R1 状态行 / R2 目录即状态 / R3 supersede 指向真实编号 / R4 索引完整 / R5 索引无悬挂 /
   R6 编号唯一 / R7 索引不重复）；`scripts/verify-decisions.ps1` 删掉全部自建判据，只剩转发；
   入口 `npm run decisions:check`（只读）与 `npm run decisions:fix`（`--fix-index` 机械回填，
   只搬 H1 标题、不代写摘要）。
2. **pre-commit 跨平台且 fail-closed**：有 `node` 走 node，无 node 有 `pwsh` 走 ps1 转发，
   两者皆无 ⇒ 打印原因并 `exit 1` 拒绝提交。门禁缺位不再是「静默放行」的合法状态。
3. **`npm run decisions:check` 插进 `npm run verify`**（版本同步之后、单测之前）：治理门与构建门
   同一条链，不再依赖开发者记得单独跑。
4. **撞号消解规则（补进 `docs/decisions/README.md`）**：同一编号被发现对应两个文件时，
   **被外部引用更多的一篇保留原号**，另一篇顺延到下一个空闲号（本例：wave-35 测试套件修复
   `0038` → `0110`；TUI 设计系统保留 `0038`），slug 不变，旧号永不回用；改号只动文件名与 H1，
   **正文一字不改**。
5. **封存件不回改**：`management/task-system/99_LOGS/`、`.dev-team/contracts/wave-36-pa138-145.md`、
   `.dev-team/state-archive-through-wave41.json` 里写作 `0038-wave35-suite-remediation.md` 的历史引用
   保持原样——它们是当时事实的收据，改写它们等于伪造历史；对照关系写进被改号那篇正文顶部的
   「编号修订」块，grep 旧号或旧路径都能命中该块。只改**存活指针**：`docs/architecture/plan-execution-approval.md`
   与 README 索引行。
6. **ADR 与其索引行同提交**：本篇与 `0110` 的索引行随代码一起落地；尚未落地的 ADR（`0101`/`0105`/
   `0106`/`0107`/`0108`）的索引行**不得提前进入** HEAD，否则 R5 在新克隆上判红。
   同理，门禁落地的那次提交必须让**它自己在干净克隆上判绿**：`0099` 的 `Status: accepted` 与
   `0104` 的 `Status: proposed`（主目录）两处状态行随本提交一并修正——各 1 行，不触碰其正文决定。
7. **`write-adr` skill 撤掉自建 grep 判据**，改指向同一入口，并把「取号前先验撞号」写进取号步骤。

## 候选方案（Alternatives considered）

- **A. 继续 ps1 单实现，要求 Linux 开发机装 pwsh**——落选：把治理门禁押在解释器安装上，
  没装就等价于「这道门不存在」，而本次 27 条违规正是这段历史的直接产物。
- **B. ps1 与 mjs 双实现并存、各查各的**——落选：两套判据必然漂移（mjs 已经多出 R4–R7），
  漂移的代价由下一个提交者付；`check-decisions.mjs` 头部注释把这条写成了明文禁令。
- **C. node 单一判据 + ps1 转发 + 两者皆缺 fail-closed**——**采纳**：本机与 Windows 侧共用同一份
  判定；代价是提交者必须有 node（本仓 `npm test` 已假设 node 存在，不构成新门槛）。
- **D. 只在 CI 跑，不进 pre-commit**——落选：违规 ADR 会先落进 `main` 再红，回滚一条已合并的
  治理事实比本地拦下贵一个量级；本地门为纯文本毫秒级检查（109 篇实测 50ms），预算内。
- **E. 撞号时改 TUI 设计系统那篇（`0038` → 新号）**——落选：保留哪篇的判断依据是引用面，
  `0038-tui-design-system` 被 `0042`/`0043`/`0076`/`0078`/`0079`、`docs/INDEX.md` 与 wave-39 文档
  作为视觉语汇反复互引（8+ 处存活引用），而 wave-35 那篇的存活引用只有 2 处
  （`plan-execution-approval.md`、README 索引）；让引用少的一方改号，改动面最小。
- **F. 给旧号留一个 tombstone 文件**——落选：新增一个 `0038-*.md` 会再次触发 R6（同一编号两文件），
  且违反「同一编号只允许对应一个文件」这条既有纪律。

## 影响

- 提交路径多了一道秒级门；任何 clone 只要 `git config core.hooksPath .githooks` 生效，
  目录/状态/索引/编号四类漂移都无法静默进入 `main`。
- **不可逆面**：`0110` 一旦被引用就不得再改号；`0038` 这个号在本仓从此只指 TUI 设计系统一篇。
- **恢复条件**：若某日需要在无 node 也无 pwsh 的环境提交，唯一出路是把该环境的 node 装上或
  显式 `--no-verify`（会绕过全部门禁，须由 review 追责），不为门禁另写第三份实现。
- 门禁只做文本级判定，不理解语义：摘要质量、候选方案是否编造仍**靠 review**——这条边界写在这里，
  免得把「门变绿」当成「治理合格」。

