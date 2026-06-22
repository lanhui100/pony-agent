# PA-059 实现与收口日志

## 日期
2026-06-22

## 任务
PA-059: 扩展 TurnHistoryMessage 携带消息元数据

## 方式
分阶段实现 → 每阶段子智能体审核 → 采纳调优 → 最终 3 路并行代码审核 → 再次调优 → 收口

## 实现阶段

| 阶段 | 内容 | 审核 | 结果 |
|---|---|---|---|
| Stage 1 | Rust 数据模型：MessageStatus 枚举、TurnHistoryMessage 扩展 5 字段 + PartialEq,Eq + stable_id() + 32 处构造修复 | ✅ 通过 | 无问题 |
| Stage 2 | Rust 投影逻辑：enrich_history_from_traces（末端对齐）、extract_reasoning_content、derive_status_from_trace、snapshot_from_state 双路径 | ✅ 通过 | 无问题 |
| Stage 3 | 前端 TS 类型：TurnHistoryMessage 追加 5 字段 | ✅ 通过 | 无问题 |
| Stage 4 | 前端 hydration：hydrateMessagesFromHistory 优先级重构、buildTurnHistory 传播、isPersistedMessageShapeCompatible 简化 | ✅ 通过 | 无问题 |
| Stage 5 | 新增测试：4 个 Rust + 1 个前端 | ✅ 通过 | 初始 snapshot 测试因无 turn_trace_history 失败，改为直接构造 SessionState 后通过 |

## 最终代码审核（3 路并行）

| 审核维度 | 发现问题 | 采纳 |
|---|---|---|
| Rust 正确性 | 无严重问题 | — |
| 前端正确性 | 无问题 | — |
| 集成/QA | 缺 checkout_history_node_metadata 测试、stable_id() serde(skip) 语法错误、mixed 测试覆盖不足 | 全部修复 |

## 最终验证

```powershell
# Rust 测试（全部通过）
cargo test -p pony-agent-core --lib -- serde_roundtrip_enriched_metadata                # ✅
cargo test -p pony-agent-core --lib -- serde_roundtrip_old_blob_compatible              # ✅
cargo test -p pony-agent-core --lib -- snapshot_enriched_history_metadata               # ✅
cargo test -p pony-agent-core --lib -- snapshot_mixed_metadata_preserves_existing        # ✅
cargo test -p pony-agent-core --lib -- checkout_history_node_metadata                   # ✅

# 现有回归测试（全部通过）
cargo test -p pony-agent-core --lib -- file_backend_roundtrip_restores_sessions         # ✅
cargo test -p pony-agent-core --lib -- file_backend_roundtrip_restores_turn_trace_history # ✅

# 前端测试
npx vitest run tests/runtime-store.spec.ts        # 76/79 passed（3 个 pre-existing failure）
  # ✅ hydrates message metadata from history when snapshot carries enriched TurnHistoryMessage
  # ✅ hydrates reasoning, model label, and tool history from trace when persisted cache is unavailable
  # ✅ hydrates assistant history from snapshot content instead of stale persisted markdown text
```

## 变更文件清单

| 文件 | 变更类型 |
|---|---|
| `crates/pony-agent-core/src/agent/session.rs` | 修改：新增 MessageStatus、TurnHistoryMessage 扩展、投影 enrich、5 个测试 |
| `src/types/runtime.ts` | 修改：TurnHistoryMessage 类型扩展 |
| `src/stores/runtime.ts` | 修改：hydrateMessagesFromHistory、buildTurnHistory |
| `tests/runtime-store.spec.ts` | 修改：新增 hydrate 测试 |
| `management/task-system/02_REVIEWS/2026-06-22-pa059-spec-review.md` | 新增：审核记录 |
| `management/task-system/03_TASKS/PA-059-spec.md` | 修改：根据审核调优 |
| `management/task-system/03_TASKS/PA-059-extend-turn-history-message-with-message-metadata.md` | 修改：status→Done |

## 关键决策
1. 索引策略从 `i/2` 改为末端对齐（处理 history 24 条与 turn_trace_history 24 条截断步长不一致）
2. `status` 从 `Option<String>` 改为 `Option<MessageStatus>` 枚举（消费者精确类型契约）
3. `isPersistedMessageShapeCompatible` 不简化为恒 `true`（`id` 仍需要 persisted 或合成）
4. 新增 `stable_id()` 为所有消费者提供统一消息标识符
5. `#[serde(skip)]` 在方法上不可用，改为注释说明（方法本身不参与序列化）
