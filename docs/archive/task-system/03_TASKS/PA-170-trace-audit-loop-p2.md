# PA-170: Trace 审核 Loop P2 —— 一次唤起整轮推进 + patrol 巡检

## 状态
- **状态**: 待启动（依赖 PA-169）
- **优先级**: P2
- **Complexity**: B
- **创建日期**: 2026-11-08
- **前置依赖**: PA-169（节点脚本化完成且维度 6 机械判 pass）
- **决策依据**: [ADR 0101](../../../docs/decisions/0101-trace-audit-loop-automation.md)

## 需求背景

loop 的中期性要求：用户任意一次唤起即机械推进到下一停点，无需逐步人工驱动；七维度队列走完后转入 patrol 回归巡检，使审核成为持续机制而非一次性工程。

## 交付契约

- **C1 整轮推进**：单次唤起（交互 skill 或 `ponycli` headless 单命令）自当前 phase 推进至下一停点（await_approval / escalated / 预算耗尽），全程 CAS + lease + 回执。
- **C2 审批门机械检查**：implement 迁移前必须验证提案 frontmatter `approved`；未批准实施 = 门禁红（防作弊核心断言）。
- **C3 patrol 模式**：queue 耗尽后 mode=patrol；每次唤起跑棘轮全集 + corpus 重放对当前 release；回归 → 受影响维度自动开新 round（mode=queue）。
- **C4 升级处置**：attempt ≥3 / rejected → escalated，人工重置前唤起 no-op；通知走 send-wecom-notification.mjs。
- **C5 预算**：每次唤起 timeout-secs 上界 + 治理成本窗口（ADR 0048）约束。

## 非目标

- 定时器/cron 无人值守（可选演进，不列验收）。
- CI 编排。

## 验收标准（物理收据）

1. 连续 3 次唤起均机械推进至下一停点，回执链完整。
2. 零未批准实施：构造 proposed 状态下唤起，implement 不发生且门禁可证。
3. 零假 PASS：构造 UNKNOWN 证据，verdict 不 pass。
4. patrol 回归开轮：人为破坏已过维度断言目标后唤起，patrol 检出并开新 round。