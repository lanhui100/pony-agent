# RETIRED: docs/decisions 历史决策体系已退役并冻结

> **Status: retired**  
> **退役时间**: 2026-11-11  
> **决策依据**: [`.agents/notes/implemented/simplification/2026-11-11-retire-docs-decisions-to-notes.md`](../../../.agents/notes/implemented/simplification/2026-11-11-retire-docs-decisions-to-notes.md)  
> **后续跟踪**: `backlog:B102`（全面审核 docs/archive/decisions 归档 ADR：评估重构为 .agents/notes 规范并迁移）

---

## 退役说明

本目录（原 `docs/decisions/`，现已移入 `docs/archive/decisions/`）包含仓库早期采用的数字编号线性递增架构决策记录（`0001` 至 `0123` 篇）。随着 Ponygo 标准治理框架与 `.agents/notes/`（生命周期 `proposed/implemented/rejected/archived` + 领域类别 `feature/bug-fix/architecture/...`）体系的全面落地，本目录已正式完成历史使命，**整体退役并冻结为只读历史档案**。

### 决策单一真源说明

为彻底消除 ADR 双轨并存导致的事实多源、心智负担与双重门禁开销，项目技术决策体系已全面统一收口：

1. **唯一现行决策真源**：
   - 路径：[`.agents/notes/`](../../../.agents/notes/)；
   - 规范与格式：按 [`.agents/notes/README.md`](../../../.agents/notes/README.md) 编写，包含 `## Problem`、`## Decision`（现在时）、`## Alternatives considered`、`## Reintroduction conditions` 等；
   - 门禁看守：由 `.meta/gates/adr-verify.sh`、`.agents/skills/write-adr/verify-note.sh` 以及 `.meta/gates/ci-verify.sh` 进行 1.1~1.11 项严格机械检查。
2. **历史档案只读保留**：
   - 本目录下现存的 `0001` 至 `0123` 篇历史 ADR、`README.md` 及子目录（`proposed/`, `superseded/`, `rejected/`）保持原貌只读保留，用于历史考古溯源与跨文件防断链保护；
   - **禁止在此新增任何新的 ADR 或编辑既有文档**。
3. **存量重构与迁移机制**：
   - 已在活账本中立项 `backlog:B102`，后续将系统性逐篇审核归档中的 ADR，对当前仍具长效架构指导意义的决策，按照 `.agents/notes` 规范重构并迁移至新体系中。
