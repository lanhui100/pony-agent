# Design

## Decision Summary

1. 新增 `turn_events` 表（append-only）：`(session_id, turn_id, branch_id, seq, event_type, payload, created_at_ms)`，`seq` 会话内全局单调、从 0 起、恒等于已落盘事件数（0-based；`seq` 分配由 `SqliteSessionBackend` 连接 Mutex 单写者保证——流式 emit 发生在 move 闭包内不持有 `sessions_rwlock`）。
2. 事件类型定义：Rust enum `TurnEvent` + 各 payload struct（serde），`domain/action` 命名；构造/反序列化失败 fail loud（数据完整性错误），持久化写入失败 contained（IO 错误，只记日志不阻断流）。
3. **缓冲模型**：`emit_event` 双路改造——推送前端（现有路径不动）+ 事件入内存缓冲；turn 终态（completed/failed/cancelled）统一 flush 与 blob 快照写入同一事务。mid-turn 崩溃丢弃未 flush 缓冲是接受的降级。
4. **事件版本策略**：`event_schema_version`（`store_metadata`，单整数）；结构变更才 bump，新增事件类型不 bump；未知必需事件 fail loud，未知 ignorable 事件可跳过（借鉴 dsh `SESSION_FORMAT_VERSION` + `ignorable`）。
5. chunk 压缩 = **写时聚合**：emit 时按 `(turn_id, step)` 合并到内存缓冲，落盘时每逻辑事件一行、seq 连续；不做事后合并（与 append-only 不可变 + seq 连续一致）。
6. 事件层永不截断；归档 = 同库归档表 `turn_events_archive`（保留 seq 与 PK，折叠引擎透明读取），不在本 change 实现仅定案语义。
7. 迁移回填：从 blob 反推事件（承认 `DEFAULT_HISTORY_LIMIT` 截断导致**只回填最近 24 个 turn** 的数据损失）；逐 session 幂等（per-session 回填水位）。

## Chosen Direction

### 1. 表结构

```sql
CREATE TABLE IF NOT EXISTS turn_events (
  session_id    TEXT NOT NULL,
  turn_id       TEXT NOT NULL,
  branch_id     TEXT NOT NULL DEFAULT 'main',  -- 阶段 3 起分支可见性推导的基石
  seq           INTEGER NOT NULL,              -- 会话内全局单调，从 0 起
  event_type    TEXT NOT NULL,
  payload       TEXT NOT NULL,                 -- lossless JSON（含 schema_version 冗余字段）
  created_at_ms INTEGER NOT NULL,
  PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_turn_events_turn ON turn_events (session_id, turn_id);
CREATE TABLE IF NOT EXISTS turn_events_archive (  -- 定案语义，本 change 不实现归档迁移
  session_id    TEXT NOT NULL,
  turn_id       TEXT NOT NULL,
  branch_id     TEXT NOT NULL,
  seq           INTEGER NOT NULL,
  event_type    TEXT NOT NULL,
  payload       TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  archived_at_ms INTEGER NOT NULL,
  PRIMARY KEY (session_id, seq)
);
```

- 与 `sessions` 表同库同连接（复用 `SqliteSessionBackend` 单连接 + WAL）。
- `seq` 分配：`store_metadata` 计数器（key = `turn_event_seq:<session_id>`），**与事件写入同一事务**递增；崩溃一致性由事务回滚保证（计数器与事件行同生共死）。计数器缺失（legacy）时以 `MAX(seq)+1` 修复。
- `event_schema_version` 存 `store_metadata`（key = `event_schema_version`）；写入时随 payload 冗余 `schema_version` 字段（读侧校验）。

### 2. 事件类型定义

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    TurnStart { turn_id: String },
    TurnEnd { turn_id: String, reason: TurnEndReason, turn_duration_ms: Option<u64> },
    StepStart { turn_id: String, step: u32, first_token_latency_ms: Option<u64> },
    StepEnd { turn_id: String, step: u32 },
    UserMessage {
        turn_id: String,
        text: String,
        attachments: Vec<AttachmentReference>,
    },
    AssistantChunk { turn_id: String, step: u32, text: String },
    AssistantMessage {
        turn_id: String,
        step: u32,
        text: String,
        reasoning_content: Option<String>,
        usage: Option<TokenUsage>,        // 仅消息元数据展示，不参与 MetricsProjection totals
        chunk_missing: Option<bool>,      // 回填/降级标记
    },
    ToolCall {
        turn_id: String,
        step: u32,
        call_id: String,
        name: String,
        arguments: String,
        started_at_ms: Option<u64>,       // TurnToolActivity.duration_seconds 重建所需
    },
    ToolResult {
        turn_id: String,
        step: u32,
        call_id: String,
        result: Option<String>,           // 32KB 截断在投影层做，事件层存引用或截断标记
        error: Option<String>,
        status: Option<String>,           // done/error
        duration_ms: Option<u64>,
        artifacts: Option<Vec<Value>>,
        capability_invocation: Option<CapabilityInvocationRecord>,
    },
    PlanUpdate { plan_state: Value },
    /// 每次 provider 调用结算（ProviderCallCacheRecord 的事件化）。
    /// 缓存命中（cache_hit/cache_miss）只有 provider 结算时才知道，必须进事件，
    /// 否则 ADR 0007 的缓存命中一等指标无法从事件重建。
    ProviderUsage {
        turn_id: String,
        step: u32,                        // 初始请求=0，每次 tool followup +1（唯一键）
        request_kind: ProviderRequestKind,
        usage: TokenUsage,                // input/cache_read/cache_write/output
        cache_hit_input_tokens: Option<u64>,
        cache_miss_input_tokens: Option<u64>,
        prefix_mutation_reasons: Vec<String>,   // 与 ProviderCallCacheRecord 对齐
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,    // 本次调用耗时（非 turn 耗时；turn 耗时以 turn/end 为准）
        latency_kind: ProviderLatencyKind,
        provider: String,
        model: String,
    },
    /// 上下文压缩：投影丢弃 base_seq 之前的消息事件并注入摘要（折叠不复活被压缩历史）。
    HistorySquash { base_seq: u64, summary_message: String },
    CheckpointCreated { node_id: String, kind: String, parent_node_id: Option<String> },
    CheckpointCheckout { node_id: String, mode: String },
    ForkCreated { branch_id: String, from_node_id: String },
}
```

- 事件类型是**编译期契约**（Rust enum 天然封闭，比 dsh 的声明合并更严格——插件扩展后续再议）。
- `payload` 列存 `serde_json::to_string(&event)` 的完整事件（含 type tag），`event_type` 列冗余索引用。
- **指标随事实走**：token usage 携带在 `assistant/message`，延迟携带在 `step/start`/`turn/end`，provider 结算独立 `ProviderUsage` 事件（dsh token-meter 模式：指标是 log 的投影，不是独立存储）。
- **计数规则（防双计数）**：`MetricsProjection` 的 totals 只累计 `ProviderUsage`；`assistant/message` 的 usage 仅用于消息元数据展示。

### 3. emit_event 双路改造（缓冲模型）

`turn_flow.rs:500` 的 `emit_event(sink, name, payload)`：

```
emit_event(sink, name, payload)
    ├─▶ 现有：sink.emit(name, payload) 推送前端（不动）
    └─▶ 新增：sink.persist(事件) —— 默认空实现，生产 sink 入内存缓冲
```

- **注入机制**：扩展 `TurnEventSink` trait 增加 `persist(&self, event: TurnEvent)`（默认空实现）；生产 sink（`runtime/mod.rs` 持 buffer）覆写；测试 `RecordingTurnEventSink` 覆写收集——避免改 27+ 个调用点签名。
- **事件名 → variant 映射**（8 种现有发射名 + 新增 emit 点）：

| TurnStreamEvent.kind | TurnEvent variant | 来源 |
|---|---|---|
| `turn:started` | `TurnStart` | 现有 emit |
| `turn:delta`（chunk） | `AssistantChunk` | 现有 emit |
| `turn:completed` | `AssistantMessage` + `TurnEnd`（reason=completed）+ `ProviderUsage`（结算点） | 现有 emit + 结算点 |
| `turn:failed` / `turn:cancelled` | `TurnEnd`（reason=error/aborted） | 现有 emit |
| `turn:output_end` | 不落盘（纯前端展示标记）或 `StepEnd` | 阶段 1 不落盘，后续再议 |
| `turn:tool`（call/completed） | `ToolCall` / `ToolResult` | 现有 emit + 工具执行路径补充字段 |
| `turn:trace` | 不落盘（trace 是投影不是事实） | — |
| `turn.context_built` | 不落盘（前端展示；`build_context_observation` 阶段 4 外置） | — |
| 新增 | `StepStart` / `StepEnd` | 新增 emit 点（各 hop 边界，runtime/mod.rs 各 hop 处） |
| 新增 | `UserMessage` | `append_turn` 的 user 消息处 |
| 新增 | `PlanUpdate` | `plan_state.rs` 更新处（阶段 1 可豁免，plan 状态暂不事件化） |
| 新增 | `HistorySquash` | `replace_session_history` 处 |

- **缓冲语义**：生产 sink 维护 per-turn 缓冲（`Vec<TurnEvent>` + 上限告警）；turn 终态（`turn:completed/failed/cancelled`）flush——缓冲事件 + blob 快照**同一事务**落盘；mid-turn 崩溃丢缓冲（接受的降级，spec 显式声明）。
- **持久化失败**：`eprintln!`/logger 记录 `(session_id, turn_id, event_type, seq)`，不抛错不阻断流；该事件标记 `persist_failed`（审计闭环：失败行数 == 日志记录数）；重试按原 seq 补写（阶段 1 仅记录，重试机制后续）。
- **session_id 为 None**（`emit_stream_failed` 可传 None）：归属 `DEFAULT_SESSION_ID` 并告警，或丢弃并告警（二选一：选**丢弃并告警**，避免污染默认会话）。

### 4. turn 终态 flush 与快照同事务

- 扩展 `PersistCommand`：新增 `FlushEvents { epoch, session_id, turn_id, events: Vec<TurnEvent> }`。
- `SqliteSessionBackend` 在**同一事务**内：写 `turn_events` 行（batch）+ upsert `sessions.session_data`。
- epoch 语义沿用：旧 epoch 命令在 materialize 后拒绝（迁移 barrier）。
- 顺序固定：先事件后快照（事件是事实，快照是投影，事实先落）。
- **测试注入点**：`write_full_store` 增加 `#[cfg(test)]` 注入开关（事件行已写、快照未写之间强制失败），验证事务回滚（两表均无残留）。

### 5. 事件层不截断 + 写时聚合

- `DEFAULT_HISTORY_LIMIT` 只作用于 blob 快照的 `history` 字段，`turn_events` 永不删除。
- chunk 压缩 = **写时聚合**：emit 时同一 `(turn_id, step)` 的连续 chunk 在内存缓冲合并为一条 `AssistantChunk`（payload 存拼接文本 + 起止时间）；**每逻辑事件占一个 seq、一行**，落盘后不可变。不做事后合并/行修改。
- 归档（`turn_events_archive`）语义定案：按 `created_at_ms` 区间搬移，保留 seq 与 PK；折叠引擎读写时透明访问主表 + 归档表。本 change 不实现归档迁移。

### 6. 迁移回填

- 一次性脚本/启动时迁移（复用 `migrate_from_json` 模式）：遍历 `sessions` 表，从 blob 的 `history` + `turn_trace_history` 反推事件。
- **数据损失承认**：blob 的 `history`/`turn_trace_history` 被 `DEFAULT_HISTORY_LIMIT=24` 截断，回填只能反推**最近 24 个 turn**；更早 turn 无事件，标记 `backfill_partial` 于 `store_metadata`。
- 反推规则：
  - `history` 中 user 消息 → `UserMessage`；assistant 消息 → `AssistantMessage`（含 `chunk_missing: true`）+ `TurnEnd`；turn 边界按 user 消息切分（无法配对的消息按"单条消息 = 独立 turn"合成，标记 `turn_boundary_synthetic`）；
  - `turn_trace_history` 的 `tool_activities` → `ToolCall` + `ToolResult`（缺 started_at/duration 的字段为 None）；
  - 无 chunk 过程 → `AssistantMessage` 携带 `chunk_missing: true`；
  - `provider_call_records` → `ProviderUsage`（usage 缺失时以 turn 级 token 字段反推，缺 input/cache 桶为 None——MetricsProjection 对 legacy 的显式降级语义）。
- **幂等**：`store_metadata` 记录 **per-session** 回填水位（`turn_event_backfill:<session_id>` 存最大回填 seq）；回填与正常写入共用同一 seq 计数器事务；中途崩溃重跑时已完成 session 跳过、未完成 session 从断点继续。

## Verification Strategy

- 单元：事件序列化 round-trip（语义等价 + 字段级一致，非 byte-identical）、seq 连续性（0-based、并发 Barrier 双线程各 100 条无空洞）、turn 终态 flush 事务原子性（`#[cfg(test)]` 注入点回滚断言）、回填逐 session 幂等、写时聚合。
- 集成：真实 turn 运行后对比 `turn_events` 与 `TurnStreamEvent` 推送序列（表驱动：8 种事件名逐一断言映射）。
- 回归：既有 session/trace/checkpoint 测试全绿；验收命令用 Rust 测试（`npm run cargo:test`），`npm run verify`（仅前端单测 + cargo check）**不等于** Rust 回归通过。