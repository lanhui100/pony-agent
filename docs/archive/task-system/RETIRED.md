# RETIRED: management/task-system 任务系统已退役并冻结

> **Status: retired**  
> **退役时间**: 2026-11-11  
> **决策依据**: [`.agents/notes/implemented/process/2026-11-11-retire-legacy-task-system.md`](../../../.agents/notes/implemented/process/2026-11-11-retire-legacy-task-system.md)

---

## 退役说明

本目录（原 `management/task-system/`，已移入 `docs/archive/task-system/`）作为早期多智能体协作与手工 Markdown 任务看板系统，已正式完成历史使命，**现已移出工作区根目录并整体退役冻结为只读历史档案**。

### 职责分流与真源说明

为彻底消除与 `backlog/` 的命名/概念混淆，并杜绝与 ADR 体系（最高优先级决策真源）的事实多源与知识腐化，本目录职责已正交分流如下：

1. **决策层（Why & Contract）**：
   - 唯一真源：[`.agents/notes/`](../../../.agents/notes/) 与 [`docs/decisions/`](../../decisions/)；
   - 凡涉及架构设计、非平凡变更、技术规范、替代方案权衡，一律编写 ADR，受 `check-decisions` 与 `adr-verify` 门禁约束。不再创建 `03_TASKS/` 卡片与 `02_REVIEWS/`。
2. **执行层（Who & When & State）**：
   - 唯一真源：[`backlog/tasks.jsonl`](../../../backlog/tasks.jsonl)；
   - 凡日常待办、缺陷（Defect）、代码债务（Debt）与开发任务（Task），一律通过 `node scripts/backlog.mjs` 推进，受行级强校验、原子租约并发锁（`backlog/.locks/`）与 `backlog lint` 门禁约束。
3. **历史层（History）**：
   - 本目录下现存的 `00_DASHBOARD.md`、`01_TASK_BOARD.md`、`02_REVIEWS/`、`03_TASKS/`、`99_LOGS/` 保持原样只读保留，用于历史考古溯源与相对路径防断链保护；
   - **禁止在此新增或修改任何任务卡、评审文件或状态流**。

---

## 存量任务迁移对账

- **已完成项结单归档**：`PA-164`、`PA-171`、`PA-172`、`PA-114`、`PA-148`、`PA-149~152`、`PA-077` 代码已实质落地，已在看板标明收口，不占活待办。
- **活跃待办迁入 Backlog**：
  - `PA-170`（Trace 审核 Loop P2） $\rightarrow$ `backlog:B083`
  - `PA-165`（ACP 宿主接入） $\rightarrow$ `backlog:B084`
  - `PA-166`（跨进程 SDK） $\rightarrow$ `backlog:B085`
  - `PA-167`（无头授权缺口） $\rightarrow$ `backlog:B086`
  - 存量技术债与缺陷（PA-119, 115, 116, 117, 120, 121, 122, 123, 134, 146, 147, 135, 136, 130, 131） $\rightarrow$ `backlog:B087~B101`
- **放弃项（Dropped）**：
  - `PA-132`（设计令牌迁移）、`PA-133`（Retina 发丝边框变体）标记为 Dropped。
- **远期规划留档**：
  - `PA-044`、`PA-022`、`PA-026`、`PA-104~111` 留存架构设计文档作为路线图愿景，待实际启动时再行立项与立 ADR。
