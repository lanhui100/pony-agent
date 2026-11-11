# Agent Note: docs/decisions 决策目录退役并统一收口至 .agents/notes

Status: implemented

## Problem

在引入 Ponygo L2 架构治理规范后，仓库同时存在两套并行的架构决策记录（ADR）体系：

1. **双轨并存与事实多源**：
   - 早期历史决策体系位于 `docs/decisions/`（采用 `0001` 至 `0123` 线性数字递增编号），由自研脚本 `scripts/check-decisions.mjs` 校验目录、状态与 README 索引；
   - 现行治理规范位于 `.agents/notes/`（采用 `{lifecycle}/{class}/yyyy-mm-dd-topic-title.md` 分层），由 `.meta/gates/adr-verify.sh` 和 `.agents/skills/write-adr/verify-note.sh` 守护；
   - 开发者与智能体在沉淀决策时面临双轨选择困难与心智割裂，经常出现重复记录或格式漂移。
2. **双重门禁维护成本与 CI 耗时**：
   - Pre-commit 钩子中既执行 Ponygo 标准门禁 `adr-verify.sh`，又执行 `check-decisions.mjs`；
   - `package.json` 中的 `verify` 任务重复绑定了多套校验，造成冗余开销。
3. **顶层入口不收敛**：
   - `AGENTS.md` 和 `docs/AGENTS.md` 中长期同时罗列两套路径，未能达成「每个事实一个家」的工程宪章。

## Decision

1. **docs/decisions/ 移出主视线并归档至 docs/archive/decisions/**：
   - 整体移动既有 `docs/decisions/` 目录至 `docs/archive/decisions/`，移出顶层活跃视线，降低智能体工作区搜索时的语义熵增；
   - 新增 `docs/archive/decisions/RETIRED.md` 并标记 `Status: retired`，原样冻结保留 `0001` 至 `0123` 篇历史 ADR、子目录及 `README.md`，用于考古溯源与历史提交/代码防断链；禁止在此新增或编辑文件。
2. **决策真源唯一确立为 .agents/notes/**：
   - 所有现行架构设计、非平凡变更与技术规范，一律按 `.agents/notes/` 契约编写并存放在对应生命周期与类别目录下；
   - 决策门禁统一收口至 Ponygo 官方标准的 `adr-verify.sh` 与 `verify-note.sh`。
3. **门禁与测试解耦优化**：
   - 清理 `.githooks/pre-commit` 中对 `check-decisions.mjs` 的直接调用，完全由 `adr-verify.sh` 接管暂存区 ADR 校验；
   - 更新 `package.json`，将 `verify` 任务中的 `decisions:check` 升级为 `adr:check`（执行 `verify-note.sh`）；
   - 将 `scripts/check-decisions.mjs` 的默认检测目录重定向至 `docs/archive/decisions`，并将 `tests/decisions-gate.spec.ts` 改造为只读守护归档区一致性。
4. **顶层地图与技能库重定向**：
   - 同步更新 `AGENTS.md`、`docs/AGENTS.md` 与 `.meta/docs-tier/README.md`，彻底将 ADR 唯一路径确立为 `.agents/notes/`；
   - 将技能库（`streaming-motion`、`trace-audit-loop` 等）中的旧决策链接重定向至归档目录。
5. **立项长效审核与重构（B102）**：
   - 在活账本中登记任务 `backlog:B102`，对归档中的 123 篇历史 ADR 进行系统性审核，将仍具长效指导意义的核心决策逐步重构为 `.agents/notes/` 规范并迁移。

## Alternatives considered

- **A. 彻底物理删除 docs/decisions/**：
  落选——`0001`~`0123` 包含仓库长达数月的演进决策（事件溯源、TUI 架构、安全白名单、流式动效等），直接物理硬删除会导致历史代码注释、提交日志与跨文档引用大面积断裂；移入 `docs/archive/decisions/` 归档冻结既能消除根部视野污染，又能保全证据链。
- **B. 就地保留并标记只读（不移入 archive）**：
  落选——保留在 `docs/` 活跃目录下仍会持续污染文件树检索，使后续智能体难以分清哪套才是活跃决策真源。
- **C. 一次性将 123 篇历史 ADR 全部批量重命名并改写为 notes 格式**：
  落选——旧版 ADR 格式与 Ponygo L2 的 1.1~1.11 机械格式判据差异极大（标题格式、正文时态、备选节要求等），一次性机械迁移会导致大量格式报错，甚至曲解历史决定的真实语境。通过 `B102` 按需、分期精细化审核迁移是更稳妥务实的工程路径。

## Consequences

- **物理变化**：
  - `docs/decisions/` 整体移动至 `docs/archive/decisions/`；
  - 新增 `docs/archive/decisions/RETIRED.md`，更新 `README.md` 头部声明退役；
  - `package.json` 引入 `adr:check` 并替换 `verify` 链条；
  - `.githooks/pre-commit` 清除冗余调用；
  - `AGENTS.md`、`docs/AGENTS.md`、`.meta/docs-tier/README.md` 顶层地图同步收口；
  - `backlog/tasks.jsonl` 登记 `B102` 审核重构待办。
- **行为变化**：
  - 开发者与智能体不再纠结双轨 ADR，后续非平凡决策一律按 `write-adr` 规范落入 `.agents/notes/`；
  - 暂存区与 CI 校验速度提升，消除了冗余的旧脚本重复扫描。

## Reintroduction conditions

满足任一即可评估复活：(a) 团队决定彻底废弃 Ponygo 治理框架及其 L2 notes 分层模型，全面回归单层递增数字文件池管理；(b) 引入了必须基于数字自增目录结构的第三方只读静态站点生成器。

## Removed surface

`docs/decisions/` 活跃目录结构与日常编写面（已移至 `docs/archive/decisions/` 冻结只读）。

生产消费者检索收据（零活跃写入面）：
- `git grep "docs/decisions" -- ':!docs/archive' ':!docs/decisions' ':!backlog'`：活跃顶层文档与门禁已全部重定向，剩余仅存为代码历史注释与旧测试用例中的溯源说明，无任何正在运行的生产写逻辑。
