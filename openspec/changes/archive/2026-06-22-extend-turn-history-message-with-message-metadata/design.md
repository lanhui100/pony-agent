# Design: 扩展 TurnHistoryMessage 携带消息元数据

## 背景

当前 `TurnHistoryMessage` 只携带 `role`、`content`、`attachments` 三个字段，前端在重建 `ChatMessage` 时需要从三个来源拼合。

这种多源重建导致了 `isPersistedMessageShapeCompatible` 兼容层的存在，也使得撤回到 checkpoint 后元数据容易丢失。

## 设计目标

1. `SessionSnapshot.history` 自带完整消息信息，消除前端对 localStorage 缓存的 `ChatMessage` 元数据依赖
2. 向后兼容已有 `session_data` JSON blob（新增字段为 `Option` + `#[serde(default)]`）
3. 投影层支持两条路径：live session 路径与历史节点路径
4. 所有消费者（桌面 GUI / TUI / CLI / HTTP API）共用一致的 snapshot

## 非目标

- 不把 `ChatMessage` 搬进 Rust — `TurnHistoryMessage` 仍是轻量投影
- 不改变 `turn_trace_history` 的 schema — trace 仍是独立持久化的权威数据
- 不改变 `HistoryNode.history` 的存储格式 — checkpoint 节点存储不变
- 不变更前端 `ChatMessage` 类型
- 不新增 `id` 字段到 `TurnHistoryMessage`（id 为前端概念，仍通过 persisted 或合成获取）

## 关键边界

### 1. 数据模型

新增 `MessageStatus` 枚举（`Done | Error`，`#[serde(rename_all = "snake_case")]`），`TurnHistoryMessage` 扩展 5 个 `Option` 字段：

| 字段 | Rust 类型 | 序列化名 |
|---|---|---|
| `turn_id` | `Option<String>` | `turnId` |
| `status` | `Option<MessageStatus>` | `status`（`"done"` / `"error"`） |
| `model_name` | `Option<String>` | `modelName` |
| `token_count` | `Option<u64>` | `tokenCount` |
| `reasoning_content` | `Option<String>` | `reasoningContent` |

所有新增字段标记 `#[serde(default, skip_serializing_if = "Option::is_none")]`。

### 2. 索引策略

`session.history` 按消息截断（`DEFAULT_HISTORY_LIMIT = 24` 条，约 12 turns），`session.turn_trace_history` 按 turn 截断（24 条）。超过 12 turns 后二者步长偏移，不能用 `i / 2` 索引匹配。

改用由 `turn_trace_history` 驱动的并行迭代，从末端对齐两条数组。

### 3. reasoning_content 提取

`TurnTraceRecord` 无顶层 `reasoning_content` 字段。从 `trace_timeline` 中反向查找最后一条 `call_model` 类型 entry 的 `reasoning_content`，与前端 `traceReasoningContent` 逻辑一致。

### 4. projection 幂等性

节点投影路径：当 `history` 条目已携带元数据（新版本创建的节点），不应覆写。幂等守卫：`if msg.turn_id.is_some() { continue; }`。

### 5. 前端 hydration 变更

`hydrateMessagesFromHistory` 优先级改为：
- `turnId` / `modelName` / `tokenCount` / `reasoningContent` / `status`：history item > persisted > trace
- `id`：仍从 persisted 获取，不可用时 fallback 到合成 id `"history-{role}-{index}"`

`isPersistedMessageShapeCompatible` 移除了 metadata 对比逻辑，保留 id 获取路径（不简化为恒 `true`）。

`buildTurnHistory` 从 `ChatMessage` 传播新字段到 `TurnHistoryMessage`，使浏览器预览模式的 `createSnapshotFromRuntimeState` 也能携带元数据。

### 6. 消费者标识符

新增 `stable_id()` 方法：`format!("{}-{}", self.turn_id.as_deref().unwrap_or("unknown"), self.role)`。所有消费者（桌面/TUI/CLI/HTTP）可用的稳定消息标识符，不参与序列化（方法，非字段）。

## 兼容性

| 场景 | 旧 JSON blob | 新 JSON blob | 行为 |
|---|---|---|---|
| 旧 blob + 新 Rust | 反序列化 OK（新字段 default） | — | history 元数据为 None |
| 新 blob + 旧 Rust | — | 反序列化 OK | 旧 Rust 忽略元数据 |
| 文件 roundtrip | 无变化 | 序列化含新字段 | 无 breakage |
| status enum | `"status": "done"`（字符串） | `"status": "done"`（枚举 snake_case 相同） | serde 自动兼容 |

## 验证策略

- `serde_roundtrip_enriched_metadata`：全字段 roundtrip + stable_id()
- `serde_roundtrip_old_blob_compatible`：旧 JSON blob 反序列化 + skip_serializing
- `snapshot_enriched_history_metadata`：实时路径 enrich
- `snapshot_mixed_metadata_preserves_existing`：幂等性（空 trace + 有 trace）
- `checkout_history_node_metadata`：checkout 路径 enrich
- 前端 hydrate 测试：snapshot.history 自带元数据时 hydrate 优先从 history 读取

## 风险与收敛

### 风险 1：history 与 turn_trace_history 截断步长不一致

收敛方式：末端对齐偏移策略 + `saturating_sub` 安全处理。

### 风险 2：ChatMessage.id 无后端等价字段

收敛方式：保留 persisted 用于 id 获取，不可用时使用合成 id。浏览器预览模式无 persisted 时通过 `createSnapshotFromRuntimeState` 自行合成。

### 风险 3：旧 blob 加载后 status 为 None

收敛方式：前端 `hydrateMessagesFromHistory` 在 `status` 为 `None` 时按原有逻辑推断（通过 trace phase 推导），不影响用户体验。
