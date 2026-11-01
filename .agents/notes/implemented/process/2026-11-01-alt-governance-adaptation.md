# ADR 替代形态适配与治理流向分工规范

Status: implemented

## Context
本项目原先基于 `docs/decisions/` 沉淀了大量存量架构与设计决策（0001 至 0053）。在引入 `ponygo` 元框架进行工程质量管理与治理审计时，ponygo 要求决策目录落盘于 `.agents/notes/` 并按四态生命周期及类别分类管理。为了保持历史决策的血缘与稳定性，同时严格对齐 ponygo 的质量审计自洽要求，特制定替代治理形态分工规范。

## Decision
1. **存量决策目录只读**：`docs/decisions/` 目录下既有的 0001 至 0053 等全部历史决策原样保留，作为历史只读档案，不进行批量重命名、改写或物理迁移。
2. **新增决策流向**：自本规范生效起，所有新建技术与架构决策一律落入 `.agents/notes/{lifecycle}/{class}/` 目录体系，严格遵守 ponygo 四态（proposed/implemented/rejected/archived）与六类（feature/bug-fix/simplification/architecture/process/testing）封闭集规范。
3. **门禁与校验契约**：启用 `.meta/gates/adr-verify.sh` 与相关 pre-commit / CI 门禁，对 `.agents/notes/` 中的新增及修改决策执行格式、时态、文件名、Status 及备选方案（Alternatives considered）的硬门禁校验。
4. **常载命约协同**：项目既有的 `AGENTS.md` 治理总则与 ponygo 常载命约在 `AGENTS.md` 中合并维护，保留各层事实清晰的分层地图。

## Alternatives considered
- **全量迁移存量决策至 `.agents/notes/`**：将 `docs/decisions/` 下 50 余篇历史决策脚本化迁移重命名。落选理由：破坏既有 Git Blame 历史，可能造成跨仓库与文档引用的超链接断链，违背「既有资产只读、新增按新规范演进」的防腐化原则。
- **完全维持 `docs/decisions/` 且不落地 `.agents/notes/`**：仅靠静态配置声称豁免。落选理由：无法满足 ponygo L2 自洽要求中的机械化 ADR 校验真源依赖，导致 CI 与治理工具链持续报错。

## Consequences
- 彻底解除 ponygo audit 与 status 中的替代形态 WARN，满足 L1/L2 自洽判定。
- 保证历史决策的稳定可读与未来治理规范的统一演进。
