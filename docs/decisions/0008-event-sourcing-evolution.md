# 0008 会话数据架构向事件溯源演进（分阶段落地）

Status: implemented

## 背景

Pony Agent 的会话数据目前是"状态快照"模型：`SessionSnapshot` 整包 JSON blob 存于 SQLite（`sessions.session_data`），trace 独立表迁移中（`SeparateTraceTableMode`），checkpoint 以 `HistoryNode` 内嵌完整快照。与 DeepSeek Harness（dsh）的事件溯源架构对比后（见 `docs/analysis/deepseek-harness-architecture-comparison-2026-08-14.md` 及 2026-08-18 的深入对比讨论），确认现状存在四类结构性问题：

### 1. 无单一真源，多副本靠调用顺序同步

- 消息快照（blob）、trace 表、前端内存状态是三个副本；
- 每 turn 多次整包 upsert（`append_turn`、`record_turn_trace`、hook traces…），一致性依赖调用顺序而非结构保证；
- 渲染状态与持久化状态可能不一致，且无机制检测。

### 2. 过程丢失，不可重放

- `DEFAULT_HISTORY_LIMIT` 截断 history（`session.rs`），截断后旧 turn 只剩摘要；
- turn 内中间状态（流式 chunk、工具调用时序、plan 更新）只存在于 `TurnStreamEvent` 推送，落盘后丢失；
- 无法从任何持久化数据重建 UI 状态或审计完整过程。

### 3. checkpoint 是快照复制，不是引用

- `HistoryNode` 内嵌完整快照（history + transcript + trace + memory），每次 `commit_history_node_from_live_state` 复制一份全量数据，节点越多膨胀越严重；
- 撤回（`checkout_history_node`）是"状态替换"（`hydrate_session_from_node` 复制粘贴），不是指针移动；
- fork 是复制不是引用；
- `cursor_version` 乐观锁是手工 CAS，事件溯源中天然免费。

### 4. trace 与消息分离，是结果快照而非过程

- `TurnTraceRecord` 的 `trace_steps` 只有 id/label/state 三字段，中间过程（chunk、工具调用时序）不落盘；
- trace 表权威后，消息在 blob、trace 在表，两套数据。

## 候选方案

### 方案 A：LangGraph 式 state 更新（最小改动）

把 `SessionState` 显式化为 state 对象，turn 内每个阶段更新后：内存 state 更新 → 通过现有 `TurnStreamEvent` 推送"state 更新"事件 → 随快照落库。

- 优点：改动小，前端体验立即提升。
- 缺点：turn 中途崩溃丢中间状态；不可重放；多副本问题依旧（与 LangGraph 同病）。

### 方案 B：dsh 式全量事件溯源（重写）

完整引入 append-only 事件日志作为唯一真源，快照全部降级为投影缓存，前端渲染从事件派生。

- 优点：架构最正，可重放可审计，单一真源。
- 缺点：改动最大，需要事件模型设计、版本契约、重放引擎；与现有代码的衔接成本高。

### 方案 C：混合演进（选定）

保留现有快照/trace 表/HistoryNode 结构，但**新增 append-only 事件层作为真源**，现有结构逐步降级为投影缓存。分四个阶段推进，每阶段可独立交付。

- 优点：复用现有基建（`TurnStreamEvent` 已是事件雏形、`PersistCommand` 已是命令式写入雏形、`normalized_*` 表已是投影缓存雏形）；不搞大爆炸迁移；每阶段有独立价值。
- 缺点：迁移期存在事件层与快照层双写；演进周期长。

## 决策

选定**方案 C：混合演进**。核心原则：

1. **不重写，翻转**：现有 blob 快照、trace 表、HistoryNode 全部保留，但降级为投影缓存；新增 append-only 事件层作为真源。
2. **复用已有基建**：`TurnStreamEvent`、`PersistCommand`、`normalized_*` 表、`SeparateTraceTableMode` 迁移机制都是演进起点，不是另起炉灶。
3. **每阶段可独立交付**：不搞大爆炸迁移，每阶段有独立验收标准。

### 阶段 1：turn 内事件日志（核心变革）

- 新增 `turn_events` 表（append-only）：`(session_id, turn_id, seq, event_type, payload, created_at_ms)`，`seq` 会话内全局单调、从 0 起、恒等于已落盘事件数（0-based，`seq` 分配由持久化层单写者保证，非 `sessions_rwlock`）；
- 事件类型定义（Rust enum + struct，借鉴 dsh 的 `domain/action` 命名与 `SessionEventMap` 声明合并思路）：`turn/start`、`turn/end`（含 reason + turn_duration_ms）、`step/start`（含 first_token_latency_ms）、`step/end`、`user/message`、`assistant/chunk`、`assistant/message`（含 usage）、`tool/call`、`tool/result`、`plan/update`、`provider/usage`、`history/squash`（上下文压缩事件，投影丢弃 base_seq 前消息）、`checkpoint/created`、`checkpoint/checkout`、`fork/created`；事件携带 `branch_id`（阶段 3 起分支场景可见性推导的基石）；
- **指标随事实走**：token usage 携带在 `assistant/message`，延迟携带在 `step/start`/`turn/end`，provider 结算独立 `provider/usage` 事件（含缓存命中 cache_hit/cache_miss、延迟、latency_kind、prefix_mutation_reasons——ADR 0007 缓存命中一等指标的事件化，借鉴 dsh `token-meter` 的 log 投影模式）；totals 只累计 `provider/usage`，`assistant/message` usage 仅作消息元数据展示（避免双计数）；
- **事件持久化时机 = 缓冲模型**：流式 emit 的事件先入内存缓冲，turn 终态（completed/failed/cancelled）统一 flush 并与 blob 快照写入同事务；mid-turn 崩溃丢弃未 flush 缓冲是接受的降级（与"过程不丢失"目标在崩溃窗口内的取舍显式化）；
- **事件版本策略**：`event_schema_version`（单整数，结构变更才 bump，新增事件类型不 bump），未知必需事件 fail loud、未知 ignorable 事件可跳过（借鉴 dsh SESSION_FORMAT_VERSION + ignorable 机制）；
- **写入路径**：`turn_flow.rs` 的 `emit_event` 已是统一事件出口，改造为"推送前端 + 入事件缓冲"双路；turn 终态 flush 时与 blob 快照写入**同一事务**（复用 `PersistCommand` 的 epoch 机制），迁移期双写；
- 事件层**永不截断**；归档 = 同库归档表（保留 seq 与 PK，折叠引擎透明读取）；`DEFAULT_HISTORY_LIMIT` 只作用于快照。

### 阶段 2：投影层——快照降级为缓存

- 定义投影 trait（借鉴 dsh 的 init/apply/view）：`HistoryProjection`、`TraceProjection`、`PlanProjection`、`MetricsProjection`（token usage / 缓存命中 / 延迟四桶 totals + last + per-turn 聚合，addReplacing 语义）；
- `append_turn` 从"push 消息 + 整包快照"改为"append 事件 + 增量折叠投影"，消除整包 upsert 写放大；
- `record_turn_trace` 从"组装后存表"改为"事件折叠"，独立 trace 表降级为投影的持久化缓存（带 seq 水位，重放不回归）；
- 验收：删除事件后从事件重建的快照与 blob 快照逐字段一致（对拍测试）。

### 阶段 3：checkpoint 引用化——撤回/fork 的 dsh 化

- `HistoryNode` 新增 `event_seq_range` 引用，内嵌快照字段标记 deprecated 仅 legacy 兜底；
- 撤回（checkout）= append `checkpoint/checkout` 事件 + 投影水位回退到目标节点的 `event_seq_range.end`，O(1) 无数据复制；
- fork = 事件前缀引用（类似 dsh 的 `seedLength`），分支共享事件只 fork 水位；
- 时间旅行：任意节点 = 折叠到对应 seq；
- `cursor_version` 乐观锁退役（seq 水位本身就是版本）。

### 阶段 4：trace 事件化（收尾）

- `TurnTraceRecord` 从独立权威表变为 `TraceProjection` 的缓存；
- `trace_timeline` 从事件折叠（`step/start` → `call_model`、`tool/call` → `call_tool`、`tool/result` → `return_result`）；
- 前端 trace 视图与消息视图从同一份事件派生，不再两套；
- `build_context_observation` 等大字段保持事件外置策略（事件里存引用）。

## 结果

这个决策意味着：

- Pony Agent 的会话数据从"状态快照"模型逐步演进为"事件溯源 + 投影缓存"模型；
- 单一真源、过程可追溯、渲染与持久化不可能不一致——三个领域的顽疾（多副本同步、过程丢失、checkpoint 膨胀）在演进完成后一次性消失；
- 演进是分阶段的，每阶段独立交付独立验收，不阻塞现有功能开发；
- 迁移期双写（事件 + 快照），阶段 2 快照改增量折叠后写放大消失。

一句话概括：

**不重写，翻转**——把"状态"降级为"投影"，把"事件"提升为"真源"，分四个阶段完成，每阶段可独立交付。