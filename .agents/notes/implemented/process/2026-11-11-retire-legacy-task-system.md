# Agent Note: management/task-system 任务系统退役（移出根目录归档至 docs/archive/，职责分流至 ADR 与 backlog）

Status: implemented

## Problem

仓库在演进中形成了两套命名与定位混淆的任务体系，同时与最高优先级的架构决策记录（ADR）存在严重的事实多源与重复记录：

1. **命名与概念严重冲突（Dual Task & Dual Backlog）**：
   - 仓库同时存在 `backlog/tasks.jsonl`（微观执行账本，带租约与校验门禁）与 `management/task-system/`（宏观任务看板与任务卡）；
   - `management/task-system/01_TASK_BOARD.md` 内部存在 `## Backlog` 段，与根目录 `backlog/` 命名碰撞；
   - 存在两套并行编号体系（`B001~B082` vs `PA-001~PA-175`），开发者与智能体在承接任务、登记进展时存在巨大心智负担和提交信息歧义。
2. **与第一优先级 ADR 体系严重重叠（事实重复与知识腐化）**：
   - 项目宪法常载命约规定，任何非平凡变更必须落盘为 `.agents/notes/` 下的 ADR（第一优先级决策真源）；
   - `management/task-system/03_TASKS/*.md` 任务卡与 ADR 格式高度雷同（均包含背景、设计、替代方案、契约、测试验收），导致一次开发需要在任务卡与 ADR 中双写；
   - `management/task-system/02_REVIEWS/*.md` 与当前 ADR 的三路对抗审查、复盘收据机制重复。严重违反了「每个事实一个家」的治理宪章。
3. **前置依赖已退役（框架失锚）**：
   - `management/task-system/` 早期与 `openspec` 深度绑定（其 README 中包含大量 OpenSpec 流程约定）；
   - 随着 `openspec` 在提交 `38d5c052` 中正式退役（`openspec/RETIRED.md`），`task-system` 的规范依赖已空转，且手工维护 Markdown 看板无法防御并发冲突和状态漂移（实测 `PA-164`、`PA-171`、`PA-172` 等早已合入主干的代码在看板中仍被当做 Backlog 滞留）。
4. **根目录结构污染与智能体检索语义熵增**：
   - `management/` 长期直接驻留工作区根目录并在 `AGENTS.md` 顶层地图中展示，导致后续 Agent 探索工作区时高频索引到陈旧的历史任务卡与过时方案，引发检索熵增与陈旧方案幻觉。

## Decision

1. **移出工作区根目录并归档至 docs/archive/（彻底解耦与防熵增）**：
   - 将工作区根目录下的 `management/` 整体移出根目录，归档至 `docs/archive/task-system/`、`docs/archive/reviews/`、`docs/archive/screenshots/`；根目录彻底消除 `management/` 目录；
   - 新增 `docs/archive/task-system/RETIRED.md`，并在 `docs/archive/task-system/README.md` 头部声明 `Status: retired`，禁止新增任务卡、评审记录或修改活跃状态；保留历史文件用于考古溯源与相对路径防断链。
2. **顶层地图彻底解绑，仅保留 ADR 决策**：
   - 从根目录 `AGENTS.md` 与 `docs/AGENTS.md` 顶层 Tier 索引地图中彻底移除对 `management/task-system/` 的入口登记，后续智能体日常导航不再感知该目录；
   - 仅通过本 ADR 保留治理决策链与溯源上下文。
3. **职责分流单一真源化（三层正交模型）**：
   - **决策层（Why & Contract）**：唯一收口至 **ADR 体系**（`.agents/notes/` + `docs/decisions/`），负责背景、权衡、技术契约与架构决策；
   - **执行层（Who & When & State）**：唯一收口至 **Backlog 账本**（`backlog/tasks.jsonl`），负责状态机流转、并发租约锁控（`backlog/.locks/`）、闭环证据与机械门禁（`backlog lint`）；
   - **历史层（History）**：`docs/archive/task-system/` 与 `openspec/` 永久只读冻结，仅作考古溯源。
4. **存量遗留项盘点与定向分流**：
   - **已落地项结单（7 项）**：`PA-164`、`PA-171`、`PA-172`、`PA-114`、`PA-148`、`PA-149~152`、`PA-077` 事实已入主干，在看板结单归档，不导入账本；
   - **在飞与核心特性迁入（4 项）**：`PA-170`（Trace 推进）、`PA-165`（ACP 协议）、`PA-166`（跨进程 SDK）、`PA-167`（无头授权接缝）录入 `backlog/tasks.jsonl`，编号分配至 `B083~B086`；
   - **存量缺陷与债务迁入（15 项）**：`PA-119`、`PA-115`、`PA-116`、`PA-117`、`PA-120`、`PA-121`、`PA-122`、`PA-123`、`PA-134`、`PA-146`、`PA-147`、`PA-135`、`PA-136`、`PA-130`、`PA-131` 录入 `backlog/tasks.jsonl`，编号分配至 `B087~B101`；
   - **低收益项放弃（2 项）**：`PA-132`（设计令牌层迁移）、`PA-133`（Retina 边框变体）标记为 `Dropped`；
   - **远期规划留档（11 项）**：`PA-044`、`PA-022`、`PA-026`、`PA-104~111` 留存架构设计文档作为路线图参考，不占执行账本。

## Alternatives considered

- **A. 根目录就地冻结归档（原初提案）**：
  在 `management/` 下增加 `RETIRED.md` 并标记退役，保留在根目录。
  落选——根目录下保留庞大的 Markdown 任务系统和截图目录，会持续污染 Agent 的工作区搜索（文件树扫描、全文 grep 等），容易引发陈旧上下文注入与幻觉。
- **B. 彻底物理删除 `management/`（无归档）**：
  落选——该目录下包含数十篇早期架构审查记录（`02_REVIEWS/`）与任务复盘日志（`99_LOGS/`），直接物理删除会丢失宝贵的历史推演脉络和证据链。
- **C. 最终折中方案（选定）：移出根目录归档至 `docs/archive/` + 从 `AGENTS.md` 彻底移除入口**：
  既完整保全了历史考古资产与已有文档引用的可溯源性，又从智能体的“视野中枢”（根目录与 `AGENTS.md`）彻底清除了干扰源，达成零检索熵增。
- **D. 将历史 135 项 PA 任务全量导入 `backlog/tasks.jsonl`**：
  落选——绝大多数历史 PA 任务早已结单，全量导入会向活账本注入大量历史冗余，破坏账本轻量性。

## Consequences

- **物理变化**：
  - 根目录 `management/` 整体移动至 `docs/archive/`（包含 `task-system/`, `reviews/`, `screenshots/`），根目录彻底消除 `management/`；
  - 新增 `docs/archive/task-system/RETIRED.md`，并在 `docs/archive/task-system/README.md` 标记退役状态；
  - `docs/archive/task-system/01_TASK_BOARD.md` 状态与对应条目打上结单/迁移标注并冻结；
  - `backlog/tasks.jsonl` 录入 19 条有效待办（B083~B101），`.next-id` 推进至 102；
  - `AGENTS.md` 与 `docs/AGENTS.md` 彻底移除 `management/` 入口；
  - 适配测试 `tests/dashboard-claims-consistency.spec.ts` 与 `scripts/audit/scope_check.py` 的归档路径。
- **行为变化**：
  - 新增任何需求或缺陷一律使用 `node scripts/backlog.mjs record`（不产生 Markdown diff 噪音）；
  - 非平凡技术决策一律编写 ADR（`.agents/notes/`），不再创建 `03_TASKS/` 卡片；
  - 任务执行统一走 `backlog start`（原子租约锁控）与 `backlog resolve`（闭环证据校验）；
  - Agent 不再受到根目录历史任务卡与已废弃方案的干扰。

## Reintroduction conditions

满足任一即可评估复活：(a) 引入了全新的跨团队项目管理 SaaS，且具备机器双向同步能力；(b) 团队明确要求脱离 Git 文本治理，全面恢复基于庞大 Markdown 目录的流水账记录模式。

## Removed surface

`management/` 根目录结构及 `management/task-system/` 的活跃任务流与看板编辑面（移至 `docs/archive/` 冻结，并从顶层地图移除）。
