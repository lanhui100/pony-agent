# PA-059 Spec: TurnHistoryMessage 扩展消息元数据

## 1. 动机

当前 `TurnHistoryMessage` 只携带 `role`、`content`、`attachments` 三个字段，前端在重建 `ChatMessage` 时需要从三个来源拼合：

```
TurnHistoryMessage.role + content       → ChatMessage.role + content
TurnTraceRecord.turnId                  → ChatMessage.turnId
TurnTraceRecord.providerName/Model      → ChatMessage.modelName
TurnTraceRecord.outputTokens            → ChatMessage.tokenCount
TurnTraceRecord.traceTimeline           → ChatMessage.reasoningContent
localStorage ChatMessage.id             → ChatMessage.id（丢失时 fallback "history-user-N"）
```

这种多源重建导致了 `isPersistedMessageShapeCompatible` 兼容层的存在，也使得撤回到 checkpoint 后元数据容易丢失。

## 2. 改动范围

### 2.1 Rust 端：`TurnHistoryMessage` 新增字段及配套类型

新增 `MessageStatus` 枚举：

```rust
// session.rs
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Done,
    Error,
}
```

`TurnHistoryMessage` 新增字段（均为 `Option`，向后兼容）：

```rust
// session.rs
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHistoryMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentReference>,

    // 新增字段（均为 Option，向后兼容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<MessageStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl TurnHistoryMessage {
    /// 所有消费者（桌面/TUI/CLI/HTTP）可用的稳定消息标识符，不参与序列化。
    /// 格式："{turn_id}-{role}"，turn_id 缺失时 fallback 为 "unknown-{role}"。
    #[serde(skip)]
    pub fn stable_id(&self) -> String {
        format!("{}-{}", self.turn_id.as_deref().unwrap_or("unknown"), self.role)
    }
}
```

所有新增字段均标记为 `#[serde(default)]`，保证能反序列化旧版 JSON blob（无迁移要求）。
`status` 从 `Option<String>` 改为 `Option<MessageStatus>` 枚举，消费者获得精确类型契约。

序列化约定：
- 所有 `Option` 字段用 `skip_serializing_if = "Option::is_none"`，旧 blob 重新序列化时不产生噪音 null
- `stable_id()` 用 `#[serde(skip)]` 标注，不进入持久化 JSON

### 2.2 投影层补全逻辑

在 `snapshot_from_state`（`session.rs:2039`）中新增 enrich 逻辑。

**设计约束**：
- `session.history` 按消息截断（`DEFAULT_HISTORY_LIMIT = 24` 条，约 12 turns），`session.turn_trace_history` 按 turn 截断（24 条）。超过 12 turns 后二者步长偏移，**不能**用 `i / 2` 索引匹配。
- 改用由 `turn_trace_history` 驱动的并行迭代：从末端对齐两条数组，再按 `(user, assistant)` 成对向前填充。

**非节点投影路径**（`node_id = None`，返回全量 live 视图）：

```rust
let msg_count = session.history.len();
let trace_count = session.turn_trace_history.len();
// 从末端对齐：最后一条 turn trace 对应最后两条 history 消息
let offset = trace_count.saturating_sub(msg_count / 2);

for (i, msg) in session.history.iter_mut().enumerate() {
    let turn_idx = i / 2;
    let trace_idx = turn_idx + offset;
    if msg.role == "assistant" {
        if let Some(trace) = session.turn_trace_history.get(trace_idx) {
            msg.turn_id = Some(trace.turn_id.clone());
            msg.model_name = trace.provider_model.clone();
            msg.token_count = trace.output_tokens;
            msg.reasoning_content = extract_reasoning_content(trace);
            msg.status = Some(derive_status_from_trace(trace));
        }
    } else if msg.role == "user" {
        if let Some(trace) = session.turn_trace_history.get(trace_idx) {
            msg.turn_id = Some(trace.turn_id.clone());
        }
    }
    // tool 角色消息不在 TurnHistoryMessage 中，不处理
}
```

**辅助函数定义**：

```rust
/// 从 trace_timeline 提取最终 assistant 消息的 reasoning_content。
/// 取最后一条 `call_model` 类型 timeline entry 的 reasoning_content（与前端 traceReasoningContent 逻辑一致）。
fn extract_reasoning_content(trace: &TurnTraceRecord) -> Option<String> {
    trace.trace_timeline.iter()
        .rev()
        .find(|e| e.entry_type == "call_model")
        .and_then(|e| e.reasoning_content.clone())
}

/// TurnTraceRecord.phase → MessageStatus 映射
/// "completed" → MessageStatus::Done
/// "failed" | "cancelled" → MessageStatus::Error
/// 其他（如 "pending"）→ 历史消息不应出现，降级为 MessageStatus::Error
fn derive_status_from_trace(trace: &TurnTraceRecord) -> MessageStatus {
    match trace.phase.as_str() {
        "completed" => MessageStatus::Done,
        "failed" | "cancelled" => MessageStatus::Error,
        _ => MessageStatus::Error,
    }
}
```

**节点投影路径**（`node_id = Some(id)`，返回 checkpoint 视图）：

`HistoryNode` 已有 `turn_trace_history: Vec<TurnTraceRecord>`，逻辑同上。注意幂等性：如果 `history` 条目已携带元数据（新版本创建的节点），不应覆写。

```rust
if msg.turn_id.is_none() {
    // 仅在快照未携带元数据时补全（向后兼容旧节点）
}
```

### 2.3 前端侧清理

#### 必要强制变更

- **`TurnHistoryMessage` TS 类型**（`src/types/runtime.ts`）：追加 5 个可选字段 `turnId?: string`、`status?: 'done' | 'error'`、`modelName?: string`、`tokenCount?: number`、`reasoningContent?: string`
- **`buildTurnHistory`**（`runtime.ts`）：从 `ChatMessage` 传播新字段到 `TurnHistoryMessage`，使浏览器预览模式的 `createSnapshotFromRuntimeState` 也能携带元数据

#### 简化清理逻辑

`hydrateMessagesFromHistory` 的新行为（签名不变但内部简化）：

```
// 新流程：
for each history item:
  - role + content + attachments 直接从 history 取（不变）
  - turnId / modelName / tokenCount / reasoningContent：优先从 history item 读
    （不再是：优先从 persisted.messages 读，fallback 到 trace）
  - id：仍从 persisted.messages 取（ChatMessage.id 没有后端等价字段）
    若 persisted 不可用，fallback 到合成 id "history-{role}-{index}"
  - status：从 history item 读（persisted 状态可能过期）
```

`isPersistedMessageShapeCompatible` 的新语义：
- **元数据部分不再依赖 persisted**——移除 metadata 对比逻辑
- **保留 id 获取路径**——persisted 仅用于恢复 `ChatMessage.id`（DOM key 稳定性）
- 浏览器预览模式无 persisted 时，使用合成 id（`history-user-N` / `history-assistant-N`）
- **不简化为恒 `true`**，仍检查角色列表兼容性以复用 id

保留的浏览器预览模式路径：
- `createSnapshotFromRuntimeState` 通过 `buildTurnHistory` 生成携带元数据的 `TurnHistoryMessage`
- `isPersistedStateCompatible` 不变，仍用于判断 localStorage 缓存有效性

### 2.4 不动的部分

- `SessionState.history` 的持久化格式不变（旧记录加载时新增字段为 `None`）
- `turn_trace_history` 的存储与读写不变
- 前端 `ChatMessage` 类型不变
- `isPersistedStateCompatible` 仍用于判断缓存是否可用（浏览器预览模式）

## 3. 兼容性

| 场景 | 旧 JSON blob | 新 JSON blob | 行为 |
|---|---|---|---|---|
| 旧 blob + 新 Rust | 反序列化 OK（新字段 `default`） | — | history 元数据为 `None` |
| 新 blob + 旧 Rust | — | 反序列化 OK（旧 Rust 忽略未知字段 + `#[serde(rename_all = "snake_case")]` 枚举序列化为字符串，旧 Rust 忽略该字段） | 旧 Rust 忽略元数据 |
| 文件 roundtrip | 无变化 | 序列化含新字段 | 无 breakage |
| status enum 旧→新 | `"status": "done"`（旧版本可能写为字符串） | `"status": "done"`（枚举 `snake_case` 序列化结果相同） | serde 自动兼容 |

## 4. 验证

1. `cargo test --lib file_backend_roundtrip_*` — 旧 blob roundtrip 不受新字段影响
2. `cargo test --lib checkout_history_node_*` — checkout 后的 history 元数据正确
3. `npm run test:unit -- --run tests/runtime-store.spec.ts` — 前端消息重建不需要 localStorage
4. 手动验证：浏览器预览模式无后端时，localStorage 仍能兜底
5. **新增 Rust 测试：** `file_backend_roundtrip_enriched_metadata` — 构造携带元数据的 `TurnHistoryMessage`，验证 roundtrip 后元数据保留
6. **新增 Rust 测试：** `checkout_history_node_metadata` — 验证 checkout 后 assistant 消息的 `status`/`model_name` 正确
7. **新增 Rust 测试：** `snapshot_mixed_metadata` — 验证部分消息有元数据、部分为 None（旧 blob 场景）的幂等性
8. **新增前端测试：** `hydrate_with_history_metadata` — `snapshot.history` 自带元数据时，hydrate 优先从 history 读取，不再 fallback 到 trace

## 5. 边界

- **不是**把 `ChatMessage` 搬进 Rust — `TurnHistoryMessage` 仍是后端的轻量投影，只是现在用 Option 字段附加 trace 中的元数据
- **不改变** `turn_trace_history` 的 schema — trace 仍然是独立持久化的权威数据
- **不改变** `HistoryNode.history` 的存储格式 — checkpoint 读取时通过投影补全，节点本身存储不变
- **`stable_id()` 不参与序列化** — `#[serde(skip)]`，只在内存投影中存在，不写入 JSON blob
- **`snapshot_from_state` 新增文档注释**：明确其为所有消费者（桌面 GUI / CLI / TUI / HTTP API）提供自包含快照，新增字段为便利投影，权威数据源仍是 `turn_trace_history`