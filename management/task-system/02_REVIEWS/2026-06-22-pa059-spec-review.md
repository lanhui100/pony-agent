# PA-059 Spec Review

## 审核对象

- [PA-059 任务卡](../03_TASKS/PA-059-extend-turn-history-message-with-message-metadata.md)
- [PA-059 Spec](../03_TASKS/PA-059-spec.md)

## 审核方式

- 模型：`opencode/deepseek-v4-flash-free`
- 方式：5 个不同角度的只读 spec 审核，不改文件

## 审核维度

1. **架构与数据模型** — `TurnHistoryMessage` 数据模型完整性、投影层索引对齐、架构边界
2. **持久化、兼容性与数据治理** — serde 注解正确性、前后端类型对齐、测试覆盖
3. **落地可行性与验收验证** — AC 可测性、`isPersistedMessageShapeCompatible` 简化可行性、`id` 处理路径
4. **agent-core 基座解耦** — core 是否混入桌面前端概念、localStorage 是否渗入 core、TUI/CLI/HTTP 消费者友好性
5. **未来消费者视角（TUI/CLI/HTTP）** — snapshot 自包含性、stable id 缺失、status 类型精度

## 问题汇总

### 高优先级（已全部采纳）

| # | 问题 | 来源 | 修复 |
|---|---|---|---|
| 1 | `i/2` 索引在 history/turn_trace_history 截断步长不一致时失效 | 审核 1, 3 | 改为末端对齐策略 |
| 2 | `reasoning_content` 提取路径未定义 — `TurnTraceRecord` 无该字段 | 审核 1 | 定义 `extract_reasoning_content` 从 `trace_timeline` 提取 |
| 3 | `derive_status_from_trace` 映射表缺失 | 审核 1, 3 | 补充 phase → MessageStatus 映射 |
| 4 | 伪代码 `trace` 变量在 `else` 分支越界 | 审核 1 | 修复为从外部的 `trace_idx` 获取 |
| 5 | `isPersistedMessageShapeCompatible` 不可能简化为恒 `true`（缺 id） | 审核 3 | 改为"保留 id 路径，元数据不再依赖 persisted" |
| 6 | `status` 为 `Option<String>` 类型不精确 | 审核 5 | 改为 `Option<MessageStatus>` 枚举 |

### 中优先级（已全部采纳）

| # | 问题 | 来源 | 修复 |
|---|---|---|---|
| 7 | TS `TurnHistoryMessage` 类型未标为必改 | 审核 2 | 补充 TS 类型变更 |
| 8 | `buildTurnHistory` 需传播新字段给浏览器预览模式 | 审核 2, 4 | 补充要求 |
| 9 | 缺少 enriched metadata roundtrip 测试 | 审核 2, 3 | 补充 3 个 Rust + 1 个前端测试 |
| 10 | `token_count` serde 不一致 | 审核 1, 2 | 统一加 `skip_serializing_if` |
| 11 | 缺 `stable_id()`，消费者各自造轮子 | 审核 5 | 新增 `#[serde(skip)]` 方法 |
| 12 | 节点投影路径幂等性未定义 | 审核 1, 3 | 补充幂等规则 |

### 低优先级（观察项，未采纳改动）

| # | 问题 | 来源 | 理由 |
|---|---|---|---|
| A | `PartialEq, Eq` 缺失 | 审核 2 | 已有 `Default`，且 struct 不是比较核心，留待后续 |
| B | `token_count` JSON number 精度 | 审核 5 | token count 不会超 2^53，暂不处理 |

## 采纳与调优

调优已全部应用至：
1. `PA-059-spec.md` — 重写 §2.1（MessageStatus 枚举、stable_id、serde 统一）、§2.2（末端对齐策略、辅助函数定义、幂等规则）、§2.3（前端清理的精确行为）、§3（兼容矩阵补 enum）、§4（新增 4 个测试）、§5（边界补充）
2. `PA-059-extend-turn-history-message-with-message-metadata.md` — 更新 11 条验收标准、补充审核记录摘要

## 未采纳项

无。本轮审核发现的所有问题均已采纳，未遗留未解决的 blocker。

## 结果

- 当前版本已从"方向正确但索引假设脆弱、前端清理过于乐观"升级为**可直接实现的 spec**
- 核心修复：索引策略、status 类型、id 处理路径、reasoning_content 提取、测试覆盖、消费者基础设施（stable_id）
- spec 现已覆盖：投影层幂等性、末端对齐索引、枚举类型契约、前后端类型同步、测试矩阵

## 审核证据

- `opencode` session：PA-059 Review Arch & Data Model
- `opencode` session：PA-059 Review Persistence & Compatibility
- `opencode` session：PA-059 Review Feasibility & Acceptance
- `opencode` session：PA-059 Review Core Decoupling
- `opencode` session：PA-059 Review Consumer Perspective
