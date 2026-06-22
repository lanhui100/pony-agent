# PA-059 扩展 TurnHistoryMessage 携带消息元数据

## 状态
- Status: `Done`
- Priority: `P3`
- Owner: `opencode`
- Dependencies: 无（独立于 PA-058 的 schema，只改投影层）

## 当前问题

`SessionSnapshot.history` 的类型是 `Vec<TurnHistoryMessage>`，只包含 `role + content + attachments`，缺少 `turnId`、`status`、`modelName`、`tokenCount`、`reasoningContent` 等消息级元数据。

## 设计目标

补齐 `TurnHistoryMessage` 的元数据字段，使前端可以从后端 `SessionSnapshot` 直接获取完整消息信息，不再依赖 localStorage 缓存的 `ChatMessage` 来补元数据。

## Spec 文档

参见 [spec.md](./PA-059-spec.md)

## 审核记录

- 日期：2026-06-22
- 方式：5 个子智能体并行审核（架构与数据模型 / 持久化兼容性 / 落地可行性与验收验证 / agent-core 基座解耦 / 未来消费者 TUI/CLI/HTTP 视角）
- 结论：`Conditionally Pass`（风险等级：Low-Medium，已通过调优解决）

### 审核采纳的关键调优

1. 索引策略从 `i/2` 改为末端对齐（处理 history 与 turn_trace_history 截断步长不一致的问题）
2. 明确定义 `reasoning_content` 从 `trace_timeline` 提取的规则，`derive_status_from_trace` 映射表
3. `status` 从 `Option<String>` 改为 `Option<MessageStatus>` 枚举，给消费者精确类型契约
4. 新增 `stable_id()` 方法为所有消费者提供统一消息标识符
5. AC5 明确 `id` 仍从 persisted 注入或合成，AC6 改为"保留 id 路径，不简化为恒 true"
6. 补充前端 TS 类型变更和 `buildTurnHistory` 传播新字段的要求
7. 补充 4 个新增测试（enriched roundtrip、checkout metadata、混合状态、前端 history 优先）
8. 所有 `Option` 字段统一 `skip_serializing_if = "Option::is_none"`
9. `snapshot_from_state` 新增文档注释，明确其作为多方消费者基座的投影语义

详细审核意见见：`02_REVIEWS/2026-06-22-pa059-spec-review.md`

## 验收标准

1. `TurnHistoryMessage` 新增 `turn_id`、`status`（`Option<MessageStatus>` 枚举）、`model_name`、`token_count`、`reasoning_content` 字段（均为可选，向后兼容），附带 `stable_id()` 方法（`#[serde(skip)]`）
2. `snapshot_for_session_at` 在投影时从 `SessionState.turn_trace_history` 补全这些字段（使用末端对齐策略，不使用 `i/2` 索引匹配）
3. `snapshot_for_session_at` 在投影节点快照时从 `HistoryNode.turn_trace_history` 补全（幂等：已携带元数据的条目不覆写）
4. 后端 `snapshot_at` 返回的 `SessionSnapshot.history` 包含完整元数据
5. `hydrateMessagesFromHistory` 可以直接从 `snapshot.history` 获取 `turnId`、`modelName`、`tokenCount`、`reasoningContent`、`status`，不再需要 `persisted.messages` 补元数据；`id` 仍从 persisted 获取或 fallback 到合成 id
6. `isPersistedMessageShapeCompatible` 移除 metadata 对比逻辑，保留 id 获取路径（不简化为恒 `true`，仍校验角色兼容性以复用 DOM key）
7. `TurnHistoryMessage` 的序列化与反序列化兼容已有 `session_data` JSON blob（新增字段为 `#[serde(default)]`，`status` 为 `snake_case` 枚举序列化后与旧字符串兼容）
8. 现有 `file_backend_roundtrip_*` 测试在无迁移的情况下仍能通过
9. 前端 `runtime-store` 测试在无 localStorage 的情况下仍能正确构建消息
10. 新增 Rust 测试：enriched metadata roundtrip、checkout 后元数据正确性、混合元数据状态幂等性
11. 新增前端测试：`snapshot.history` 自带元数据时 hydrate 从 history 读取优先于 trace
