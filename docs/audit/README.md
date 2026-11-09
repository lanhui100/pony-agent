# Trace 审核 Loop（人读契约）

中期持续的 harness 审核机制：按维度分批「审核 → 提案 → 批准 → 实施 → 构建安装 → 重放验证 → 棘轮」，由用户随时唤起，凭持久状态断点续跑。设计正典：[ADR 0101](../decisions/0101-trace-audit-loop-automation.md)。

## 唤起

任意会话说「**继续 trace 审核**」（加载 skill `.agents/skills/trace-audit-loop/`）。loop 读 `loop-state.json` 推进到下一停点即止。不依赖定时器，不要求常驻团队。

## 批准（唯一人工节点）

提案在 `rounds/<id>/proposals/<round>.md`，人工将 frontmatter `status: proposed` 改为 `approved`（或 `rejected`）后再唤起。未批准时 loop 停在 `await_approval` no-op。

## 文件地图

| 路径 | 职责 |
| :-- | :-- |
| `loop-state.json` | 单一真值源：mode/维度队列/当前 round/phase/lease/回执索引 |
| `ratchet.json` | 棘轮集：已通过维度断言全集（只增不减，ADR 0090 纪律） |
| `rounds/<id>/round-state.json` | 轮次快照：收口后归档该轮 baseline/candidate/写域/冻结哈希，供历史轮复跑裁决 |
| `corpus/` | 固定只读重放任务集 + manifest（hash 批准时冻结） |
| `rounds/<id>/` | 每轮产物：receipts / root-cause.md / proposals / metrics |
| `../../scripts/audit/` | P1 起确定性节点脚本（replay/fetch/assert/ratchet/build-install/verdict） |

## 维度队列

6 上报链路自检 → 1 时延结构 / 3 失败语义 → 2 上下文经济 → 5 成本治理 → 7 跨宿主一致性 → 4 行为质量。队列走完转 **patrol**：每次唤起跑棘轮全集 + corpus 重放，回归自动开新轮。

## 纪律

verdict 只认断言退出码；UNKNOWN ≠ PASS；每维度尝试 ≤3 次超限 escalated；阈值冻结后放宽须新提案重审。任务进度：PA-168（P0）/ PA-169（P1）/ PA-170（P2）。