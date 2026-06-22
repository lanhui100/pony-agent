# Proposal: 扩展 TurnHistoryMessage 携带消息元数据

## Why

当前 `SessionSnapshot.history` 的类型是 `Vec<TurnHistoryMessage>`，只包含 `role + content + attachments`，前端在重建 `ChatMessage` 时需从三个来源拼合：

- `TurnHistoryMessage` 提供 `role + content + attachments`
- `TurnTraceRecord` 提供 `turnId`、`modelName`、`tokenCount`、`reasoningContent`
- localStorage 缓存的 `ChatMessage` 提供 `id`（丢失时 fallback "history-user-N"）

这种多源重建导致 `isPersistedMessageShapeCompatible` 兼容层的存在，也使撤回到 checkpoint 后元数据容易丢失。

## What Changes

在 `TurnHistoryMessage` 中补齐消息级元数据字段，使后端 `SessionSnapshot` 自带完整消息信息：

- Rust 端：新增 `MessageStatus` 枚举，`TurnHistoryMessage` 扩展 5 个 `Option` 字段
- 投影层：在 `snapshot_from_state` 中从 `turn_trace_history` 补全元数据（末端对齐策略）
- 前端：TS 类型更新、`hydrateMessagesFromHistory` 优先级重构、`buildTurnHistory` 传播
- 消费者：新增 `stable_id()` 方法，所有消费者（桌面/TUI/CLI/HTTP）共用稳定标识符

## Impact

- 前端不再依赖 localStorage 缓存来补消息元数据
- `isPersistedMessageShapeCompatible` 移除 metadata 对比逻辑
- 旧 JSON blob 通过 `#[serde(default)]` 完全向后兼容
- 新增字段均在序列化时 `skip_serializing_if = "Option::is_none"`，不产生噪音 null

## Tracking

- Task card: `PA-059`
- OpenSpec Change: `extend-turn-history-message-with-message-metadata`
