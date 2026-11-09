# r1-dim6 双跑冲突记录

## 事实

| 项 | 会话 A（本记录产出方） | 会话 B |
| :-- | :-- | :-- |
| session_id | `pa_01a1ba70-a314-7712-adf2-4bf675a96f99` | `pa_01a1bae5-8b23-7dd2-8384-37dd3cdbdf3d` |
| loop-state lease | 曾持有，`taken_over_at_unix=1794138624` 被接管 | 当前持有（`expires_unix=1794140424`） |
| 产物前缀 | `*-pa01a1ba70.*` | 无后缀（`003-collect-*`、`004-collect-*`、`metrics.json`） |
| 阶段 | audit → propose（已到 `await_approval` 停点） | collect |

## 冲突处理（不越界）

1. 会话 A **未抢占** lease：检测到 lease 属于会话 B 且未过期，按 ADR 0101「双跑冲突 → 报告并 no-op」纪律执行。
2. 会话 A 的 audit/propose 产物全部加 `pa01a1ba70` 后缀，与 B 的回执序号解耦（B 已占用 `003`/`004` 序号）。
3. `loop-state.json` **未由会话 A 写入**：CAS 迁移需持有 lease，越权写状态会破坏 B 的推进。

## 证据互通

会话 B 的 `replay/base`（9 会话）与 `metrics.json` 已被会话 A 复用为审计证据，并证明**同一缺陷**：

- 全部 9 会话 `Environment: cwd=/home/dm/pony_agent`（臂 worktree 为 `/tmp/audit-r1-baseline`）；
- 全部 9 条 trace `release=null`；
- 全部 9 轮 `phase=completed` 与 `outcome=error` 并存。

即：会话 B 的 collect 证据同样受 F1（工作区绑定缺失）污染，其重放不能作为 corpus 任务的隔离证据 —— 需在 F1 修复后重跑。

## 待人工动作

1. 批准/驳回 `proposals/r1-dim6.pa01a1ba70.md`（frontmatter `status`）。
2. 决定是否让会话 B 继续持有 loop 推进权；若由会话 A 接管，需 B 释放或 lease 自然过期后 CAS 接管。
