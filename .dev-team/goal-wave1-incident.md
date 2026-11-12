# 事故记录：goal-1 波次首轮中断（2026-11-12）

## 1. 现象

上一轮（wave goal-1 首轮）在「契约矩阵冻结完成 → 派发 dev team 之前」中断，宿主工具步骤被 abort，
未产生任何代码产物。

## 2. 机器证据

| 检查 | 命令 | 结果 |
| --- | --- | --- |
| 团队看板 | `team_task_list` | `tasks: []`（无任何任务） |
| 成员花名册 | `list_agents` | 仅 `Lead`（`running`），无 teammate，状态无 `failed` |
| 目标状态 | `get_goal` | `phase=active`，`roundsStarted=0` |
| 工作区 | `git status --porcelain` | ` M scripts/trace-audit-loop.py`、`?? .dev-team/contract-matrix-goal-command.md`、`?? temp-eval-env/`、`?? tests/acceptance/stress-eval.spec.ts` |
| core 产物 | `ls crates/pony-agent-core/src/agent/goal_command.rs` | 不存在（`No such file or directory`） |

## 3. 影响面判定

- **无损**：未写入任何 Rust/TS 源码，无需回滚或 `git clean`。
- `scripts/trace-audit-loop.py`、`temp-eval-env/`、`tests/acceptance/stress-eval.spec.ts` 属于
  `verify/long-running-stress` 分支既有在途工作，与本波次无关，**保持原样不动**。
- 唯一有效产物：`.dev-team/contract-matrix-goal-command.md`（C-1..C-7 已冻结），可直接续跑。

## 4. 处置

1. 以契约矩阵为唯一真源续跑，不重新设计。
2. 本轮改为「先派发带写域隔离的团队任务，再并行推进」；每个 teammate 的写域互不重叠（见看板 `write_scopes`）。
3. 每步以命令退出码取证，禁止假设式推进。
