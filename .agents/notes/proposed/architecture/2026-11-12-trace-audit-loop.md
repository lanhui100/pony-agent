# Trace Audit Loop 推进机制设计

Status: proposed
Date: 2026-11-12

## Context
随着 Trace 审核流程的复杂化，亟需实现一次性推进与 patrol 巡检机制，以降低自动化巡检中的人工干预成本，并满足 ADR 0101 的断点续跑要求。

## Proposal
实现一套基于 Python 的 Trace Audit Loop 自动化推进脚本：
1. 单次唤起机械推进至下一停点（如 await_approval）。
2. 在队列耗尽后，自动无缝切换至 patrol 巡检模式。
3. 状态通过 `docs/audit/loop-state.json` 维护，严格执行 CAS 状态迁移机制。

## Alternatives considered
### 方案 A：每次执行都需要人工逐步交互
- **否决理由**：无法实现无头（Headless）单命令运行形态，在大规模巡检时效率低下。
