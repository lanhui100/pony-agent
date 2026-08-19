# Design

## Decision Summary

1. `HistoryNode` 新增 `event_seq_range: Option<(u64, u64)>`；内嵌快照字段保留但新节点不再写入（legacy 兜底）。
2. checkout = append `checkpoint/checkout` 事件 + 投影水位回退到 `event_seq_range.end`；无数据复制，重折叠成本显式化。
3. fork = 事件前缀共享 + 分支水位；事件携带 `branch_id`（阶段 1 已落列）。
4. **分支可见性**：投影折叠按 `checkpoint/checkout` 事件 + HistoryNode 血缘图推导可见分支集合，跳过非当前分支血缘事件。
5. 时间旅行 = 折叠到任意 seq。
6. `cursor_version` 退役：wire 新增 `event_watermark`，`expected_cursor_version` 语义改为水位校验（旧客户端不传则跳过）。

## Chosen Direction

### 1. HistoryNode 引用化

```rust
pub struct HistoryNode {
    // ... 现有字段保留 ...
    /// 该节点覆盖的事件区间（阶段 3 新增）；None = legacy（内嵌快照兜底）
    pub event_seq_range: Option<(u64, u64)>,
    // 内嵌快照字段（history/turn_trace_history/...）标记 deprecated：
    // 新节点不写，legacy 节点读取兜底
}
```

- `commit_history_node_from_live_state`：记录当前事件水位为 `event_seq_range`，不再复制快照。
- 持久化：`sqlite_session.rs` 节点行新增列（或 JSON 字段），legacy 行 `NULL` → 旧语义。

### 2. checkout 水位回退

```
checkout_history_node(node_id, mode, expected_version)
    ├─▶ 校验：目标节点存在 + 水位冲突检测（expected_event_watermark vs 当前水位）
    ├─▶ append `checkpoint/checkout` 事件（node_id, mode, branch_id）
    ├─▶ 投影水位回退：history/trace/plan/metrics 四个投影重折叠到 node.event_seq_range.end
    │     （从 init 重折叠到目标 seq；节点 → 投影状态缓存 (node_id, seq, state) 首访后复用）
    ├─▶ 缓存失效协议：删除 seq > b 的 trace/metrics 缓存行 + 增量重折叠
    │     （与阶段 2 的 higher-seq-wins 不冲突：回退是显式失效，不是重放回归）
    ├─▶ 分支截断语义保留：同分支非祖先节点标记 TurnCancelled（现有逻辑）
    └─▶ cursor 更新：visible_node_id / active_branch_id / mode=Live（现有逻辑）
```

- **metrics 同步回退**：`MetricsProjection` 缓存随水位回退（删除 seq > b 行 + 重折叠），避免"history 已回退、metrics 停在撤回前"的跨缓存发散。
- 性能：重折叠 O(目标 seq)，节点状态缓存后 O(1)；大 session 预算：10k 事件全量重折叠 < 100ms（基准断言）。
- legacy 节点（`event_seq_range: None`）：走现有 `hydrate_session_from_node` 路径，行为不变。

### 3. 分支可见性（投影折叠规则）

- 事件携带 `branch_id`（阶段 1 已落列，默认 `main`）。
- 折叠规则：投影维护"当前可见分支集合"——初始 `{main}`；`checkpoint/checkout` 事件 → 可见集合 = 目标节点所在分支的血缘链（`HistoryBranch.forked_from_*` 推导）；`fork/created` 事件 → 新分支加入候选（未 checkout 前不可见）。
- 折叠时跳过 `branch_id ∉ 可见集合` 的事件（被撤回分支的事件不复活）。
- 与"纯 apply(单事件)"契约的调和：`apply` 签名不变，投影状态携带可见集合；增量折叠时新事件若属于不可见分支则跳过（状态不变）。

### 4. fork 事件前缀共享

- fork 命令：append `fork/created` 事件（branch_id, from_node_id）+ 创建新分支（`HistoryBranch` 现有字段）。
- 新分支事件写入：同 session 事件流（`session_id` 不变），事件携带新 `branch_id`；分支水位由 `HistoryCursor.active_branch_id` + 可见集合区分。
- 备选：独立 session + `seedLength`（dsh 模式）——若产品需要跨会话隔离再引入，本 change 先同流分支。

### 5. 时间旅行

- 新命令（或复用现有查询）：`load_node_state(node_id)` → 折叠事件到 `event_seq_range.end`（按可见集合过滤）→ 返回快照。
- 前端入口（历史节点查看）后续迭代，本 change 只提供机制。

### 6. cursor_version 退役

- wire 新增 `event_watermark: u64`（`HistoryCursor` 或 `SessionSnapshot` 字段，前端可获知当前水位）。
- `expected_cursor_version` 保留字段（前端兼容），语义改为：`expected_event_watermark` 校验（`expected == current` 才放行，与旧相等比较语义一致）。
- 旧客户端不传 → 跳过校验（行为与现状一致）。
- 前端 `history.ts` 同步适配（wire 新增字段，旧字段保留）。

## Verification Strategy

- 单元：引用化（legacy 兜底）、checkout 无复制断言 + metrics 缓存同步回退、fork 共享（branch_id 过滤）、时间旅行折叠、double checkout、跨分支水位冲突、分支可见性（被撤回分支事件不复活）。
- 集成：真实会话撤回/恢复/分支切换与旧路径对拍（限定字段集）。
- 回归：既有 checkpoint/history 测试全绿；验收命令 `npm run cargo:test`。