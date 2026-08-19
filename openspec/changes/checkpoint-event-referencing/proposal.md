# checkpoint-event-referencing

## Background

阶段 2（`session-projection-layer`）落地后，快照已从事件折叠，但 checkpoint 仍是快照复制模型：

- `HistoryNode` 内嵌完整快照（history + transcript + trace + memory，`session.rs:113-150`），每次 `commit_history_node_from_live_state` 复制一份全量数据，节点越多膨胀越严重；
- 撤回（`checkout_history_node`，`session.rs:1586`）是"状态替换"（`hydrate_session_from_node` 复制粘贴），不是指针移动；
- fork 是复制不是引用；
- `cursor_version` 乐观锁（`session.rs:178`）是手工 CAS，事件溯源中天然免费。

本 change 是 ADR 0008 **阶段 3：checkpoint 引用化**——`HistoryNode` 从"内嵌快照"改为"事件 seq 区间引用"，撤回/fork 从"复制"改为"水位移动"。

## Goals

- `HistoryNode` 新增 `event_seq_range` 引用（该节点覆盖的事件区间），内嵌快照字段标记 deprecated 仅 legacy 兜底。
- 撤回（checkout）= append `checkpoint/checkout` 事件 + 投影水位回退到目标节点的 `event_seq_range.end`，O(1) 数据复制消除（重折叠成本显式化）。
- fork = 事件前缀引用（事件携带 `branch_id`，阶段 1 已落列），分支共享事件只 fork 水位。
- **分支可见性**：投影折叠时按 `checkpoint/checkout` 事件 + HistoryNode 血缘图推导"当前可见分支集合"，跳过非当前分支血缘的事件（被撤回分支的事件不复活）。
- 时间旅行：任意节点 = 折叠到对应 seq（查看任意历史状态）。
- `cursor_version` 乐观锁退役（seq 水位本身就是版本；wire 兼容方案见 design）。

## Non-goals

- 不做时间旅行 UI（本 change 只做机制，前端入口后续迭代）。
- 不做撤回的 workspace 回滚增强（`TranscriptAndWorkspace` 的 git 回滚语义保持现状——`workspace_ref` 是节点元数据，事件化后不受影响，显式声明）。
- 不做 `GraphRun` 推进事件化（run 状态、Ask 等待、plan 推进的折叠设计移出本 change scope，标注为后续 change）。
- 不改 `GraphAskWaitBinding`（保留独立表，它是跨进程握手状态不是可重放事实）。

## Scope

- `crates/pony-agent-core/src/agent/`：
  - `session.rs`：`HistoryNode` 结构扩展（`event_seq_range`）、`checkout_history_node` 重构（水位回退 + 分支可见性）、`commit_history_node_from_live_state` 重构（记录事件区间）、fork 命令重构（事件前缀引用 + branch_id）。
  - `sqlite_session.rs`：节点持久化（引用替代内嵌快照）、legacy 数据兜底读取。
  - `projection.rs`：投影折叠的分支可见性规则（跳过非当前分支血缘事件）。
- 前端：`src/lib/runtime/history.ts` 的 checkout/restore/fork 结果归一化适配（wire 格式不变则零改动）。

## Risks

- **legacy 节点兼容**：旧 `HistoryNode` 无 `event_seq_range`，撤回必须走旧路径（内嵌快照兜底）。缓解：`event_seq_range: None` → 旧语义，`Some` → 新语义。
- **水位回退的投影一致性**：撤回后投影水位回退，但事件层保留全部事件（append-only），"撤回"是视图切换不是数据删除。缓解：`checkpoint/checkout` 事件记录水位 + 分支可见性规则；**MetricsProjection 缓存同步回退**（删除 seq > b 的缓存行 + 增量重折叠），避免与阶段 2 的 higher-seq-wins 冲突。
- **分支事件归属**：事件携带 `branch_id`（阶段 1 已落列）；两分支交错 append 时，投影按当前可见分支集合过滤。缓解：折叠时跳过非当前分支血缘事件（规则见 design §3）。
- **cursor_version 退役的兼容**：前端可能仍传 `expected_cursor_version`。缓解：wire 新增 `event_watermark` 字段（前端可获知当前水位），`expected_cursor_version` 语义改为水位校验；旧客户端不传则跳过校验（行为与现状一致）。
- **重折叠成本**：checkout 从 init 重折叠 O(seq)，大 session（数千事件）成本高。缓解：节点 → 投影状态缓存（`(node_id, seq, state)`），首访重折叠后缓存。

## Validation

- 单元测试：节点引用化（legacy 兜底）、checkout 水位回退（无数据复制断言 + metrics 缓存同步回退）、fork 事件共享（branch_id 过滤）、时间旅行折叠、double checkout、跨分支水位冲突、分支可见性（被撤回分支事件不复活）。
- 集成验证：真实会话撤回/恢复/分支切换行为与旧路径对拍（限定字段集）。
- 回归：既有 checkpoint/history 测试全绿；验收命令 `npm run cargo:test`。